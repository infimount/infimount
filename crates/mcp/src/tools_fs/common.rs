use serde::Serialize;
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::policy::{evaluate_storage_policy, McpOperation, PolicyDecision};
use crate::registry::StorageRecord;
use crate::registry::StorageRegistry;
use crate::session::SessionManager;


#[derive(Debug)]
pub struct FsToolsContext {
    pub registry: StorageRegistry,
    pub sessions: SessionManager,
    pub allow_insecure: bool,
    pub auth_token: Option<String>,
}

pub struct SessionAccess {
    pub read_only: bool,
}

impl FsToolsContext {
    pub async fn validate_session(
        &self,
        session_id: Option<&str>,
        storage_name: &str,
        backend_path: Option<&str>,
    ) -> McpResult<SessionAccess> {
        let Some(session_id) = session_id else {
            return Ok(SessionAccess { read_only: false });
        };

        let can_write = self
            .sessions
            .validate_access(session_id, storage_name, backend_path)
            .await?;

        Ok(SessionAccess {
            read_only: !can_write,
        })
    }
}

pub(super) fn enforce_storage_policy(
    storage: &StorageRecord,
    backend_path: &str,
    operation: McpOperation,
    overwrite: bool,
    cross_storage: bool,
) -> McpResult<()> {
    match evaluate_storage_policy(storage, backend_path, operation, overwrite, cross_storage)? {
        PolicyDecision::Allow | PolicyDecision::RequireConfirmation { .. } => Ok(()),
    }
}


#[derive(Debug, Serialize, Clone)]
pub struct ListDirEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    File,
    Dir,
}

pub(super) fn default_limit() -> u32 {
    200
}

pub(super) fn default_read_max_bytes() -> u32 {
    262_144
}

pub(super) fn default_as_text() -> bool {
    true
}

pub(super) fn default_encoding() -> String {
    "utf-8".to_string()
}

pub(super) fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeniedDescendantBehavior {
    Filter,
    Fail,
}

pub(super) async fn collect_entries_with_policy(
    op: &opendal::Operator,
    storage: &StorageRecord,
    backend_path: &str,
    recursive: bool,
    operation: McpOperation,
    denied_behavior: DeniedDescendantBehavior,
) -> McpResult<Vec<ListDirEntry>> {
    collect_entries_inner(
        op,
        &storage.name,
        backend_path,
        recursive,
        Some((storage, operation, denied_behavior)),
    )
    .await
}

async fn collect_entries_inner(
    op: &opendal::Operator,
    storage_name: &str,
    backend_path: &str,
    recursive: bool,
    policy: Option<(&StorageRecord, McpOperation, DeniedDescendantBehavior)>,
) -> McpResult<Vec<ListDirEntry>> {
    let filter = |full_path: &str| -> infimount_core::models::Result<bool> {
        if let Some((storage, operation, denied_behavior)) = policy {
            match enforce_storage_policy(storage, full_path, operation, false, false) {
                Ok(()) => Ok(true),
                Err(error) if error.code == McpErrorCode::ERR_MCP_POLICY_DENIED => {
                    if denied_behavior == DeniedDescendantBehavior::Filter {
                        Ok(false)
                    } else {
                        Err(infimount_core::models::CoreError::Config(format!(
                            "[ERR_MCP_POLICY_DENIED] {}",
                            error.message
                        )))
                    }
                }
                Err(e) => Err(infimount_core::models::CoreError::Config(e.to_string())),
            }
        } else {
            Ok(true)
        }
    };

    let core_entries = if recursive {
        infimount_core::operations::list_entries_recursive_with_filter(op, backend_path, filter)
            .await
            .map_err(core_error_to_mcp_error)?
    } else {
        infimount_core::operations::list_entries_with_filter(op, backend_path, filter)
            .await
            .map_err(core_error_to_mcp_error)?
    };

    let out = core_entries
        .into_iter()
        .map(|e| ListDirEntry {
            name: e.name,
            path: format!("/{storage_name}/{}", e.path.trim_start_matches('/')),
            entry_type: if e.is_dir {
                EntryType::Dir
            } else {
                EntryType::File
            },
            size_bytes: if e.is_dir { None } else { Some(e.size) },
            modified_at: e.modified_at,
            etag: e.etag,
        })
        .collect();

    Ok(out)
}

pub(super) fn normalize_list_prefix(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

pub(super) fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

pub(super) async fn create_dir_chain(
    op: &opendal::Operator,
    backend_path: &str,
    storage_name: &str,
    full_path: &str,
) -> McpResult<()> {
    let trimmed = backend_path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(());
    }

    let mut current = String::new();
    for segment in trimmed.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);

        match op.stat(&current).await {
            Ok(meta) if meta.is_dir() => continue,
            Ok(_) => {
                return Err(err_with_details(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    "path already exists as a file",
                    json!({
                        "path": full_path,
                        "intermediate_path": format!("/{}/{}", storage_name, current)
                    }),
                ));
            }
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {
                let dir_path = normalize_list_prefix(&current);
                op.create_dir(&dir_path)
                    .await
                    .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
            }
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    Ok(())
}





pub(super) fn core_error_to_mcp_error(err: infimount_core::CoreError) -> crate::errors::McpError {
    match err {
        infimount_core::CoreError::Storage(e) => map_opendal_error(&e, McpErrorCode::ERR_INTERNAL),
        infimount_core::CoreError::Config(msg) if msg.contains("[ERR_MCP_POLICY_DENIED]") => {
            // This is a hack to pass through MCP policy errors that were wrapped in CoreError::Config
            // by our fallible filters. In a real system, CoreError might have a dedicated Policy variant.
            err_with_details(McpErrorCode::ERR_MCP_POLICY_DENIED, msg, json!({}))
        }
        _ => err_with_details(McpErrorCode::ERR_INTERNAL, err.to_string(), json!({})),
    }
}


pub(super) fn sort_entries(entries: &mut [ListDirEntry], recursive: bool) {
    if recursive {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        return;
    }

    entries.sort_by(|a, b| match (a.entry_type, b.entry_type) {
        (EntryType::Dir, EntryType::File) => std::cmp::Ordering::Less,
        (EntryType::File, EntryType::Dir) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
}

