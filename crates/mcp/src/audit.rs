use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::errors::{map_io_error, McpErrorCode, McpResult};
use crate::policy::McpOperation;
use crate::registry::default_config_dir;

const DEFAULT_AUDIT_LIMIT: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    Allowed,
    Denied,
    RequiresConfirmation,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: String,
    pub timestamp: String,
    pub actor_type: String,
    pub mcp_client_id: Option<String>,
    pub session_id: Option<String>,
    pub storage_id: Option<String>,
    pub storage_name: Option<String>,
    pub backend: Option<String>,
    pub tool_name: String,
    pub operation: McpOperation,
    pub path: Option<String>,
    pub version_id: Option<String>,
    pub decision: AuditDecision,
    pub matched_rule_id: Option<String>,
    pub workspace_id: Option<String>,
    pub confirmation_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub bytes_read: Option<u64>,
    pub bytes_written: Option<u64>,
    pub error_code: Option<String>,
}

impl AuditEvent {
    pub fn new(tool_name: impl Into<String>, operation: McpOperation) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            actor_type: "mcp".to_string(),
            mcp_client_id: None,
            session_id: None,
            storage_id: None,
            storage_name: None,
            backend: None,
            tool_name: tool_name.into(),
            operation,
            path: None,
            version_id: None,
            decision: AuditDecision::Allowed,
            matched_rule_id: None,
            workspace_id: None,
            confirmation_id: None,
            duration_ms: None,
            bytes_read: None,
            bytes_written: None,
            error_code: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditStore {
    path: PathBuf,
    lock_path: PathBuf,
    limit: usize,
}

impl AuditStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_audit_path);
        let lock_path = path.with_extension("lock");
        Self {
            path,
            lock_path,
            limit: DEFAULT_AUDIT_LIMIT,
        }
    }

    pub fn with_limit(path: PathBuf, limit: usize) -> Self {
        let lock_path = path.with_extension("lock");
        Self {
            path,
            lock_path,
            limit,
        }
    }

    pub fn append(&self, mut event: AuditEvent) -> McpResult<()> {
        event.path = event.path.map(|value| mask_presigned_url(&value));
        self.with_lock(|| {
            let mut events = self.load_unlocked()?;
            events.push(event);
            if events.len() > self.limit {
                let drop_count = events.len() - self.limit;
                events.drain(0..drop_count);
            }
            self.save_unlocked(&events)
        })
    }

    pub fn list_recent(&self, limit: usize) -> McpResult<Vec<AuditEvent>> {
        self.with_lock(|| {
            let mut events = self.load_unlocked()?;
            events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            events.truncate(limit);
            Ok(events)
        })
    }

    pub fn clear(&self) -> McpResult<()> {
        self.with_lock(|| self.save_unlocked(&[]))
    }

    fn load_unlocked(&self) -> McpResult<Vec<AuditEvent>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        serde_json::from_str(&data).map_err(|e| {
            crate::errors::err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse MCP audit log",
                json!({ "serde_error": e.to_string() }),
            )
        })
    }

    fn save_unlocked(&self, events: &[AuditEvent]) -> McpResult<()> {
        ensure_parent(&self.path)?;
        let payload = serde_json::to_vec_pretty(events).map_err(|e| {
            crate::errors::err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize MCP audit log",
                json!({ "serde_error": e.to_string() }),
            )
        })?;
        fs::write(&self.path, payload).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))
    }

    fn with_lock<T>(&self, f: impl FnOnce() -> McpResult<T>) -> McpResult<T> {
        ensure_parent(&self.lock_path)?;
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        lock_file
            .lock_exclusive()
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let result = f();
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

pub fn default_audit_path() -> PathBuf {
    default_config_dir().join("mcp_audit.json")
}

pub fn mask_presigned_url(value: &str) -> String {
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        return value.to_string();
    }

    let Some((base, _query)) = value.split_once('?') else {
        return value.to_string();
    };

    format!("{base}?<redacted>")
}

fn ensure_parent(path: &Path) -> McpResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn audit_store_persists_bounded_recent_events() {
        let dir = TempDir::new().unwrap();
        let store = AuditStore::with_limit(dir.path().join("audit.json"), 2);

        let mut one = AuditEvent::new("read_file", McpOperation::Read);
        one.path = Some("/Local/one.txt".to_string());
        let mut two = AuditEvent::new("read_file", McpOperation::Read);
        two.path = Some("/Local/two.txt".to_string());
        let mut three = AuditEvent::new("read_file", McpOperation::Read);
        three.path = Some("/Local/three.txt".to_string());

        store.append(one).unwrap();
        store.append(two).unwrap();
        store.append(three).unwrap();

        let events = store.list_recent(10).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|event| event.path.as_deref() == Some("/Local/two.txt")));
        assert!(events
            .iter()
            .any(|event| event.path.as_deref() == Some("/Local/three.txt")));
    }

    #[test]
    fn audit_store_masks_presigned_urls() {
        let dir = TempDir::new().unwrap();
        let store = AuditStore::with_limit(dir.path().join("audit.json"), 10);
        let mut event =
            AuditEvent::new("generate_download_link", McpOperation::PresignDownloadLink);
        event.path = Some("https://example.com/file.txt?X-Amz-Signature=secret".to_string());

        store.append(event).unwrap();
        let events = store.list_recent(1).unwrap();

        assert_eq!(
            events[0].path.as_deref(),
            Some("https://example.com/file.txt?<redacted>")
        );
    }

    #[test]
    fn audit_event_carries_matched_rule_id_and_workspace_id() {
        let mut event = AuditEvent::new("write_file", McpOperation::Write);
        event.matched_rule_id = Some("workspace:w-1".to_string());
        event.workspace_id = Some("w-1".to_string());
        event.decision = AuditDecision::Allowed;

        assert_eq!(event.matched_rule_id.as_deref(), Some("workspace:w-1"));
        assert_eq!(event.workspace_id.as_deref(), Some("w-1"));
        assert_eq!(event.decision, AuditDecision::Allowed);
    }

    #[test]
    fn audit_event_denied_carries_rule_info() {
        let mut event = AuditEvent::new("delete_path", McpOperation::Delete);
        event.matched_rule_id = Some("deny-all".to_string());
        event.decision = AuditDecision::Denied;
        event.error_code = Some("ERR_MCP_POLICY_DENIED".to_string());

        assert_eq!(event.decision, AuditDecision::Denied);
        assert_eq!(event.matched_rule_id.as_deref(), Some("deny-all"));
        assert_eq!(event.workspace_id, None);
    }

    #[test]
    fn audit_event_requires_confirmation_with_rule_and_workspace() {
        let mut event = AuditEvent::new("delete_path", McpOperation::Delete);
        event.matched_rule_id = Some("workspace:w-2".to_string());
        event.workspace_id = Some("w-2".to_string());
        event.decision = AuditDecision::RequiresConfirmation;
        event.confirmation_id = Some("confirm-123".to_string());

        assert_eq!(event.decision, AuditDecision::RequiresConfirmation);
        assert_eq!(event.workspace_id.as_deref(), Some("w-2"));
        assert_eq!(event.confirmation_id.as_deref(), Some("confirm-123"));
    }

    #[test]
    fn audit_append_sets_rule_and_workspace_fields() {
        let dir = TempDir::new().unwrap();
        let store = AuditStore::with_limit(dir.path().join("audit.json"), 10);

        let mut event = AuditEvent::new("read_file", McpOperation::Read);
        event.matched_rule_id = Some("public".to_string());
        event.workspace_id = Some("w-1".to_string());
        event.path = Some("/Local/file.txt".to_string());

        store.append(event).unwrap();
        let events = store.list_recent(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].matched_rule_id.as_deref(), Some("public"));
        assert_eq!(events[0].workspace_id.as_deref(), Some("w-1"));
    }

    #[test]
    fn audit_clear_removes_all_events() {
        let dir = TempDir::new().unwrap();
        let store = AuditStore::with_limit(dir.path().join("audit.json"), 10);

        let event = AuditEvent::new("read_file", McpOperation::Read);
        store.append(event).unwrap();
        store.clear().unwrap();
        let events = store.list_recent(10).unwrap();
        assert!(events.is_empty());
    }
}
