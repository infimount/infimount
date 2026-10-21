//! Explicitly opt-in operator pilot for the internal A4 Claude Code runner.
//! Normal release builds expose only disabled status and reject run/cancel.
//! Even a pilot cannot publish files; A1 policy, A2 evidence and output review
//! remain authoritative. The operator must acknowledge external inference.
use serde::{Deserialize, Serialize};

#[cfg(all(feature = "claude-pilot", unix))]
use std::collections::HashMap;
#[cfg(all(feature = "claude-pilot", unix))]
use std::path::PathBuf;
#[cfg(all(feature = "claude-pilot", unix))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(feature = "claude-pilot", unix))]
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePilotBuildInfo {
    pub enabled: bool,
    pub platform_supported: bool,
    pub requires_explicit_consent: bool,
    pub note: &'static str,
}

#[tauri::command]
pub fn claude_pilot_build_info() -> ClaudePilotBuildInfo {
    ClaudePilotBuildInfo {
        enabled: cfg!(all(feature = "claude-pilot", unix)),
        platform_supported: cfg!(unix),
        requires_explicit_consent: true,
        note: if cfg!(all(feature = "claude-pilot", unix)) {
            "Operator-only Claude Code qualification build; not general availability"
        } else {
            "Claude Code first-class execution is not enabled in this build"
        },
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunClaudePilotRequest {
    pub workspace_id: String,
    pub task_id: String,
    pub executable_path: String,
    pub confirm_external_inference: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelClaudePilotRequest {
    pub workspace_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePilotOutput {
    pub execution_id: String,
    pub status: infimount_core::agent_task_executions::ExecutionStatus,
    pub file_count: u32,
    pub total_bytes: u64,
    pub error_code: Option<String>,
}

#[cfg(all(feature = "claude-pilot", unix))]
type PilotKey = (String, String);
#[cfg(all(feature = "claude-pilot", unix))]
type Pilots = HashMap<PilotKey, Arc<AtomicBool>>;

#[cfg(all(feature = "claude-pilot", unix))]
fn running_pilots() -> &'static Mutex<Pilots> {
    static RUNNING: OnceLock<Mutex<Pilots>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(feature = "claude-pilot", unix))]
struct PilotLease(PilotKey);

#[cfg(all(feature = "claude-pilot", unix))]
impl Drop for PilotLease {
    fn drop(&mut self) {
        if let Ok(mut map) = running_pilots().lock() {
            map.remove(&self.0);
        }
    }
}

#[cfg(all(feature = "claude-pilot", unix))]
fn reserve_pilot(key: PilotKey) -> Result<(PilotLease, Arc<AtomicBool>), String> {
    let mut map = running_pilots()
        .lock()
        .map_err(|_| "Claude pilot cancellation registry is unavailable".to_string())?;
    if map.contains_key(&key) {
        return Err("This Agent Task already has an active local Claude pilot".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    map.insert(key.clone(), cancel.clone());
    Ok((PilotLease(key), cancel))
}

#[cfg(all(feature = "claude-pilot", unix))]
fn valid_pilot_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

#[tauri::command]
pub async fn run_claude_pilot(
    state: tauri::State<'_, crate::state::AppState>,
    request: RunClaudePilotRequest,
) -> Result<ClaudePilotOutput, String> {
    #[cfg(not(all(feature = "claude-pilot", unix)))]
    {
        let _ = (state, request);
        Err("Claude Code execution is disabled outside the opt-in Unix pilot build".into())
    }
    #[cfg(all(feature = "claude-pilot", unix))]
    {
        if !request.confirm_external_inference {
            return Err("Explicit approval is required before sharing prepared task data with Claude".into());
        }
        state.require_operational().map_err(|_| "Infimount is in restricted recovery mode".to_string())?;
        if !valid_pilot_uuid(&request.workspace_id) || !valid_pilot_uuid(&request.task_id) {
            return Err("Claude pilot requires valid task/workspace identifiers".into());
        }
        let trimmed = request.executable_path.trim();
        if trimmed.is_empty() || trimmed.len() > 2048 || trimmed.chars().any(char::is_control) {
            return Err("A safe absolute Claude Code executable path is required".into());
        }
        let executable = PathBuf::from(trimmed);
        if !executable.is_absolute() {
            return Err("A safe absolute Claude Code executable path is required".into());
        }
        let key = (request.workspace_id.clone(), request.task_id.clone());
        let (_lease, cancellation) = reserve_pilot(key)?;
        let execution_id = crate::claude_execution::execute_claude_task(
            &state,
            &executable,
            &request.workspace_id,
            &request.task_id,
            cancellation,
        )
        .await
        .map_err(|_| {
            "Claude pilot could not complete. Check task scope and private execution evidence".to_string()
        })?;
        // Return only safe A2 evidence metadata, not raw CLI output, file paths,
        // prompts, credentials, source text, or publication authority.
        let record = state.execution_records.read(
            &request.workspace_id,
            &request.task_id,
            &execution_id,
        ).map_err(|_| "Claude pilot evidence could not be verified".to_string())?;
        Ok(ClaudePilotOutput {
            execution_id,
            status: record.status,
            file_count: record.output_file_count,
            total_bytes: record.output_total_bytes,
            error_code: record.error_code,
        })
    }
}

#[tauri::command]
pub fn cancel_claude_pilot(request: CancelClaudePilotRequest) -> Result<bool, String> {
    #[cfg(not(all(feature = "claude-pilot", unix)))]
    {
        let _ = request;
        Err("Claude Code pilot cancellation is unavailable in this build".into())
    }
    #[cfg(all(feature = "claude-pilot", unix))]
    {
        if !valid_pilot_uuid(&request.workspace_id) || !valid_pilot_uuid(&request.task_id) {
            return Err("Invalid Agent Task pilot identifiers".into());
        }
        let key = (request.workspace_id, request.task_id);
        let map = running_pilots()
            .lock()
            .map_err(|_| "Claude pilot cancellation registry is unavailable".to_string())?;
        if let Some(cancel) = map.get(&key) {
            cancel.store(true, Ordering::SeqCst);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_build_is_disabled_and_pilot_is_always_consent_gated() {
        let info = claude_pilot_build_info();
        assert_eq!(info.enabled, cfg!(all(feature = "claude-pilot", unix)));
        assert!(info.requires_explicit_consent);
        assert_eq!(info.platform_supported, cfg!(unix));
    }

    #[cfg(all(feature = "claude-pilot", unix))]
    #[test]
    fn pilot_lease_prevents_duplicate_runs_and_supports_cancellation() {
        let key = (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string());
        let (lease, token) = reserve_pilot(key.clone()).unwrap();
        assert!(reserve_pilot(key.clone()).is_err());
        assert!(!token.load(Ordering::SeqCst));
        running_pilots().lock().unwrap().get(&key).unwrap().store(true, Ordering::SeqCst);
        assert!(token.load(Ordering::SeqCst));
        drop(lease);
        assert!(reserve_pilot(key).is_ok());
    }
}
