use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASKS_DIR,
    AGENT_TASK_MANIFEST_FILE,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{operations, CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use opendal::ErrorKind;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_TASK_LIST_SCAN_ITEMS: u32 = 200;
const MAX_AGENT_TASK_LIST_ITEMS: usize = 100;
const MAX_AGENT_TASK_LIST_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskListRequest {
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskSummary {
    pub task_id: String,
    pub title: String,
    pub created_at: String,
    pub task_root: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskListOutput {
    pub workspace_id: String,
    pub workspace_name: String,
    pub tasks: Vec<AgentTaskSummary>,
    pub truncated: bool,
    pub skipped_invalid_tasks: usize,
}

struct TaskListContext {
    workspace: infimount_core::workspaces::WorkspaceRecord,
    storage: StorageRecord,
    op: opendal::Operator,
}

#[tauri::command]
pub async fn list_agent_tasks(
    state: State<'_, AppState>,
    request: AgentTaskListRequest,
) -> Result<AgentTaskListOutput, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config("Agent Task listing is unavailable while Infimount is degraded".into())
    })?;

    let context = load_task_list_context(&state, &request.workspace_id)?;
    let tasks_path = join_path(&context.workspace.root_path, AGENT_TASKS_DIR);
    validate_local_path(&context.storage, &tasks_path)?;

    match context.op.stat(&tasks_path).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(CoreError::Config(
                "Agent Task directory is not a directory".into(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(AgentTaskListOutput {
                workspace_id: context.workspace.id,
                workspace_name: context.workspace.name,
                tasks: Vec::new(),
                truncated: false,
                skipped_invalid_tasks: 0,
            });
        }
        Err(error) => return Err(error.into()),
    }

    let page = operations::list_entries_page(
        &context.op,
        &tasks_path,
        MAX_AGENT_TASK_LIST_SCAN_ITEMS,
        None,
        false,
        0,
    )
    .await?;
    let mut truncated = page.truncated || page.next_cursor.is_some();
    let mut skipped_invalid_tasks = 0usize;
    let mut manifest_bytes_read = 0u64;
    let mut tasks = Vec::new();

    for entry in page.entries {
        if !entry.is_dir {
            continue;
        }
        let task_id = entry.name.trim_end_matches('/');
        if Uuid::parse_str(task_id).is_err() {
            continue;
        }

        let task_root = match agent_task_root(task_id) {
            Ok(task_root) => task_root,
            Err(_) => {
                skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
                continue;
            }
        };
        let manifest_path = join_path(
            &join_path(&context.workspace.root_path, &task_root),
            AGENT_TASK_MANIFEST_FILE,
        );
        validate_local_path(&context.storage, &manifest_path)?;

        let metadata = match context.op.stat(&manifest_path).await {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
                continue;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.content_length() > MAX_AGENT_TASK_MANIFEST_BYTES {
            skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
            continue;
        }
        let metadata_total = match manifest_bytes_read.checked_add(metadata.content_length()) {
            Some(total) => total,
            None => {
                truncated = true;
                break;
            }
        };
        if metadata_total > MAX_AGENT_TASK_LIST_MANIFEST_BYTES {
            truncated = true;
            break;
        }

        let bytes = match context.op.read(&manifest_path).await {
            Ok(bytes) => bytes.to_vec(),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() as u64 > MAX_AGENT_TASK_MANIFEST_BYTES {
            skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
            continue;
        }
        let actual_total = match manifest_bytes_read.checked_add(bytes.len() as u64) {
            Some(total) => total,
            None => {
                truncated = true;
                break;
            }
        };
        if actual_total > MAX_AGENT_TASK_LIST_MANIFEST_BYTES {
            truncated = true;
            break;
        }
        manifest_bytes_read = actual_total;

        let Ok(manifest) = serde_json::from_slice::<AgentTaskManifest>(&bytes) else {
            skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
            continue;
        };
        if validate_agent_task_manifest(&manifest).is_err()
            || manifest.task_id != task_id
            || manifest.workspace_id != request.workspace_id
            || manifest.task_root != task_root
        {
            skipped_invalid_tasks = skipped_invalid_tasks.saturating_add(1);
            continue;
        }

        tasks.push(AgentTaskSummary {
            task_id: manifest.task_id,
            title: manifest.title,
            created_at: manifest.created_at,
            task_root: manifest.task_root,
        });
    }

    tasks.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.task_id.cmp(&left.task_id))
    });
    if tasks.len() > MAX_AGENT_TASK_LIST_ITEMS {
        tasks.truncate(MAX_AGENT_TASK_LIST_ITEMS);
        truncated = true;
    }

    Ok(AgentTaskListOutput {
        workspace_id: context.workspace.id,
        workspace_name: context.workspace.name,
        tasks,
        truncated,
        skipped_invalid_tasks,
    })
}

fn load_task_list_context(
    state: &AppState,
    workspace_id: &str,
) -> Result<TaskListContext, CoreError> {
    Uuid::parse_str(workspace_id).map_err(|_| {
        CoreError::Config("Agent Task listing requires a valid workspace id".into())
    })?;
    let workspace = state
        .workspaces
        .find_by_id(workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }

    let storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(|_| {
            CoreError::Config("Agent Task workspace storage could not be validated".into())
        })?;
    if !storage.enabled {
        return Err(CoreError::Config(
            "Agent Task workspace storage must be enabled for task listing".into(),
        ));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Task listing currently requires Local Filesystem workspace storage".into(),
        ));
    }

    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".into()))?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let op = state.operator_for_storage_id(&storage.id)?;
    let capabilities = op.info().capability();
    if !capabilities.stat || !capabilities.read || !capabilities.list {
        return Err(CoreError::Config(
            "Agent Task workspace lacks required read/list capabilities for task listing".into(),
        ));
    }
    validate_local_path(&storage, &workspace.root_path)?;

    Ok(TaskListContext {
        workspace,
        storage,
        op,
    })
}

fn validate_local_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path)
        .map_err(|_| CoreError::Config("Agent Task local path confinement check failed".into()))
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim().trim_matches('/');
    let relative = relative.trim().trim_matches('/');
    match (root.is_empty(), relative.is_empty()) {
        (true, true) => String::new(),
        (true, false) => relative.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}/{relative}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_list_types_do_not_encode_source_or_publication_state() {
        let summary = AgentTaskSummary {
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            title: "Review export".into(),
            created_at: "2026-09-08T00:00:00+00:00".into(),
            task_root: "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
        };
        let json = serde_json::to_value(summary).unwrap();
        assert!(json.get("sourceStorageId").is_none());
        assert!(json.get("sourcePath").is_none());
        assert!(json.get("destination").is_none());
        assert!(json.get("published").is_none());
    }
}
