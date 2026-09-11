#![allow(non_snake_case)]

use std::path::PathBuf;

use infimount_core::workspaces::workspace_schema_supported;
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::policy::{McpAccessMode, McpRuleSource};
use infimount_mcp::registry::StorageRecord;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStorageBindingOutput {
    pub storage_id: String,
    pub normalized: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAgentAccessOutput {
    pub workspace_id: String,
    pub storage_id: String,
    pub access_profile: String,
    pub mcp_exposed: bool,
    pub changed: bool,
}

fn configured_local_root(storage: &StorageRecord) -> Option<String> {
    ["root", "rootPath", "path"].iter().find_map(|key| {
        storage
            .config
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}

fn expand_home_alias(value: &str) -> McpResult<Option<String>> {
    let trimmed = value.trim();
    let suffix = if trimmed == "~" {
        Some("")
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        Some(rest)
    } else {
        trimmed.strip_prefix("~\\")
    };
    let Some(suffix) = suffix else {
        return Ok(None);
    };
    let home = home_dir().ok_or_else(|| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "the current user's home directory could not be resolved",
        )
    })?;
    let expanded = if suffix.is_empty() {
        PathBuf::from(home)
    } else {
        PathBuf::from(home).join(suffix)
    };
    if !expanded.is_absolute() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "the resolved Local Filesystem root is not absolute",
        ));
    }
    let metadata = std::fs::metadata(&expanded).map_err(|_| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "the resolved Local Filesystem root is unavailable",
        )
    })?;
    if !metadata.is_dir() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "the resolved Local Filesystem root is not a directory",
        ));
    }
    let canonical = std::fs::canonicalize(&expanded).map_err(|_| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "the resolved Local Filesystem root could not be canonicalized",
        )
    })?;
    Ok(Some(canonical.to_string_lossy().to_string()))
}

fn normalize_local_root_config(config: &mut Value, original: &str, canonical: &str) {
    let Some(object) = config.as_object_mut() else {
        return;
    };
    for key in ["root", "rootPath", "path"] {
        let matches_original = object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == original);
        if matches_original {
            object.insert(key.to_string(), Value::String(canonical.to_string()));
        }
    }
}

#[tauri::command]
pub async fn prepare_workspace_storage_binding(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<WorkspaceStorageBindingOutput, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _configuration = state.registry.acquire_configuration_transaction()?;
    state.recover_and_require_clean_configuration_locked()?;

    let storage = state.find_storage_by_id(&storageId)?;
    if !matches!(storage.backend.as_str(), "local" | "fs") {
        return Ok(WorkspaceStorageBindingOutput {
            storage_id: storage.id,
            normalized: false,
        });
    }

    let root = configured_local_root(&storage).ok_or_else(|| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "Local Filesystem storage has no configured root folder",
        )
    })?;
    let Some(canonical) = expand_home_alias(&root)? else {
        return Ok(WorkspaceStorageBindingOutput {
            storage_id: storage.id,
            normalized: false,
        });
    };

    if state
        .workspaces
        .load_all()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to inspect bound workspaces",
            )
        })?
        .iter()
        .any(|workspace| workspace.storage_id == storage.id)
    {
        return Err(err(
            McpErrorCode::ERR_STORAGE_NAMESPACE_IN_USE,
            "legacy Local Filesystem root cannot be normalized while workspaces are already bound",
        ));
    }

    let expected_revision = storage.revision;
    state.registry.with_locked_mutation(|storages| {
        let current = storages
            .iter_mut()
            .find(|candidate| candidate.id == storage.id)
            .ok_or_else(|| err(McpErrorCode::ERR_STORAGE_NOT_FOUND, "storage was not found"))?;
        if current.revision != expected_revision {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "storage changed while preparing the workspace; retry",
            ));
        }
        let current_root = configured_local_root(current).ok_or_else(|| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "Local Filesystem storage has no configured root folder",
            )
        })?;
        if current_root != root {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "storage root changed while preparing the workspace; retry",
            ));
        }
        normalize_local_root_config(&mut current.config, &root, &canonical);
        current.revision = current.revision.saturating_add(1);
        current.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    })?;
    state.operator_cache.invalidate(&storage.id);

    Ok(WorkspaceStorageBindingOutput {
        storage_id: storage.id,
        normalized: true,
    })
}

fn expected_access(profile: &str) -> McpResult<McpAccessMode> {
    match profile {
        "read_only" => Ok(McpAccessMode::ReadOnly),
        "read_write" => Ok(McpAccessMode::ReadWrite),
        _ => Err(err(
            McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
            "workspace does not have an agent-access policy",
        )),
    }
}

fn validate_simple_workspace_exposure(
    storage: &StorageRecord,
    workspace: &infimount_core::workspaces::WorkspaceRecord,
) -> McpResult<McpAccessMode> {
    if !workspace_schema_supported(workspace) {
        return Err(err_with_details(
            McpErrorCode::ERR_WORKSPACE_SCHEMA_UNSUPPORTED,
            "workspace schema is unsupported; recreate the workspace after upgrading",
            serde_json::json!({ "workspaceId": workspace.id }),
        ));
    }
    if !storage.enabled {
        return Err(err(
            McpErrorCode::ERR_STORAGE_DISABLED,
            "workspace storage is disabled",
        ));
    }

    let access = expected_access(&workspace.access_profile)?;
    if access == McpAccessMode::ReadWrite && storage.read_only {
        return Err(err(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            "read-write agent access requires a writable workspace storage",
        ));
    }

    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(storage)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED,
                "workspace storage identity could not be verified",
            )
        })?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(err(
            McpErrorCode::ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED,
            "workspace storage identity changed; recreate the workspace before exposing it",
        ));
    }

    if storage.mcp_policy.default_access != McpAccessMode::None {
        return Err(err(
            McpErrorCode::ERR_CONFIRMATION_REQUIRED,
            "storage has broad default MCP access; review Advanced MCP settings before exposing it",
        ));
    }

    let rule_id = workspace.policy_rule_id.as_deref().ok_or_else(|| {
        err(
            McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
            "workspace MCP policy rule is missing",
        )
    })?;
    let rule = storage
        .mcp_policy
        .rules
        .iter()
        .find(|rule| rule.id == rule_id)
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
                "workspace MCP policy rule is missing",
            )
        })?;
    let source_matches = matches!(
        &rule.source,
        McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
    );
    if !source_matches
        || rule.prefix.trim_matches('/') != workspace.root_path.trim_matches('/')
        || rule.access != access
    {
        return Err(err(
            McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
            "workspace MCP policy no longer matches this workspace",
        ));
    }

    let has_broad_manual_rule = storage.mcp_policy.rules.iter().any(|candidate| {
        candidate.id != rule_id
            && matches!(&candidate.source, McpRuleSource::Manual)
            && candidate.access != McpAccessMode::None
    });
    if has_broad_manual_rule {
        return Err(err(
            McpErrorCode::ERR_CONFIRMATION_REQUIRED,
            "storage has additional manual MCP grants; review Advanced MCP settings before exposing it",
        ));
    }

    Ok(access)
}

fn load_workspace_and_storage(
    state: &State<'_, AppState>,
    workspace_id: &str,
) -> McpResult<(infimount_core::workspaces::WorkspaceRecord, StorageRecord)> {
    let workspace = state
        .workspaces
        .find_by_id(workspace_id)
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to load workspace"))?
        .ok_or_else(|| err(McpErrorCode::ERR_INVALID_PATH, "workspace was not found"))?;
    let storage = state.find_storage_by_id(&workspace.storage_id)?;
    Ok((workspace, storage))
}

#[tauri::command]
pub fn check_workspace_agent_access(
    state: State<'_, AppState>,
    workspaceId: String,
) -> Result<WorkspaceAgentAccessOutput, McpError> {
    state.require_operational()?;
    let (workspace, storage) = load_workspace_and_storage(&state, &workspaceId)?;
    validate_simple_workspace_exposure(&storage, &workspace)?;
    Ok(WorkspaceAgentAccessOutput {
        workspace_id: workspace.id,
        storage_id: storage.id,
        access_profile: workspace.access_profile,
        mcp_exposed: storage.mcp_exposed,
        changed: false,
    })
}

#[tauri::command]
pub async fn prepare_workspace_agent_access(
    state: State<'_, AppState>,
    workspaceId: String,
) -> Result<WorkspaceAgentAccessOutput, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _configuration = state.registry.acquire_configuration_transaction()?;
    state.recover_and_require_clean_configuration_locked()?;
    let _workspace_mutation = state.workspaces.acquire_mutation_lock().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to lock workspace while preparing agent access",
        )
    })?;

    let (workspace, storage) = load_workspace_and_storage(&state, &workspaceId)?;
    validate_simple_workspace_exposure(&storage, &workspace)?;

    if storage.mcp_exposed {
        return Ok(WorkspaceAgentAccessOutput {
            workspace_id: workspace.id,
            storage_id: storage.id,
            access_profile: workspace.access_profile,
            mcp_exposed: true,
            changed: false,
        });
    }

    let expected_revision = storage.revision;
    state.registry.with_locked_mutation(|storages| {
        let current = storages
            .iter_mut()
            .find(|candidate| candidate.id == workspace.storage_id)
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    "workspace storage was not found",
                )
            })?;
        if current.revision != expected_revision {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "workspace storage changed while preparing agent access; retry",
            ));
        }
        validate_simple_workspace_exposure(current, &workspace)?;
        current.mcp_exposed = true;
        current.revision = current.revision.saturating_add(1);
        current.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    })?;

    Ok(WorkspaceAgentAccessOutput {
        workspace_id: workspace.id,
        storage_id: workspace.storage_id,
        access_profile: workspace.access_profile,
        mcp_exposed: true,
        changed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::workspaces::{WorkspaceRecord, WORKSPACE_RECORD_SCHEMA_VERSION};
    use infimount_mcp::policy::{McpPathRule, McpStoragePolicy};
    use serde_json::json;

    fn workspace(storage: &StorageRecord, access_profile: &str) -> WorkspaceRecord {
        let fingerprint =
            infimount_mcp::storage_namespace::storage_namespace_fingerprint(storage).unwrap();
        WorkspaceRecord {
            id: "workspace-id".into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Workspace".into(),
            root_path: "/agent-workspaces/workspace".into(),
            template_id: "custom".into(),
            access_profile: access_profile.into(),
            policy_rule_id: Some("workspace:workspace-id".into()),
            storage_namespace_fingerprint: fingerprint,
            created_at: "2026-09-11T00:00:00Z".into(),
            updated_at: "2026-09-11T00:00:00Z".into(),
            memory_files: vec![],
            checkpoint_ids: vec![],
        }
    }

    fn storage_with_workspace_rule(access: McpAccessMode) -> (StorageRecord, WorkspaceRecord) {
        let root = tempfile::tempdir().expect("temp root");
        let mut storage = StorageRecord::new(
            "Local".into(),
            "local".into(),
            serde_json::json!({ "root": root.path().to_string_lossy() }),
        );
        let profile = if access == McpAccessMode::ReadWrite {
            "read_write"
        } else {
            "read_only"
        };
        let workspace = workspace(&storage, profile);
        storage.mcp_policy = McpStoragePolicy::default();
        storage.mcp_policy.rules.push(McpPathRule {
            id: "workspace:workspace-id".into(),
            prefix: workspace.root_path.clone(),
            access,
            source: McpRuleSource::Workspace {
                workspace_id: workspace.id.clone(),
            },
            confirmation_rules: None,
        });
        (storage, workspace)
    }

    #[test]
    fn home_alias_expands_to_an_absolute_existing_directory() {
        if home_dir().is_none() {
            return;
        }
        let expanded = expand_home_alias("~").unwrap().unwrap();
        assert!(std::path::Path::new(&expanded).is_absolute());
        assert!(std::path::Path::new(&expanded).is_dir());
    }

    #[test]
    fn ordinary_relative_root_is_not_treated_as_a_home_alias() {
        assert_eq!(expand_home_alias("relative/path").unwrap(), None);
    }

    #[test]
    fn local_root_alias_fields_are_normalized_consistently() {
        let mut config = json!({ "root": "~", "rootPath": "~", "path": "other" });
        normalize_local_root_config(&mut config, "~", "/home/example");
        assert_eq!(
            config.get("root").and_then(Value::as_str),
            Some("/home/example")
        );
        assert_eq!(
            config.get("rootPath").and_then(Value::as_str),
            Some("/home/example")
        );
        assert_eq!(config.get("path").and_then(Value::as_str), Some("other"));
    }

    #[test]
    fn scoped_workspace_policy_is_safe_to_expose() {
        let (storage, workspace) = storage_with_workspace_rule(McpAccessMode::ReadWrite);
        assert_eq!(
            validate_simple_workspace_exposure(&storage, &workspace).unwrap(),
            McpAccessMode::ReadWrite
        );
    }

    #[test]
    fn broad_default_access_requires_advanced_review() {
        let (mut storage, workspace) = storage_with_workspace_rule(McpAccessMode::ReadWrite);
        storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
        assert_eq!(
            validate_simple_workspace_exposure(&storage, &workspace)
                .unwrap_err()
                .code,
            McpErrorCode::ERR_CONFIRMATION_REQUIRED
        );
    }

    #[test]
    fn unrelated_manual_grant_requires_advanced_review() {
        let (mut storage, workspace) = storage_with_workspace_rule(McpAccessMode::ReadWrite);
        storage.mcp_policy.rules.push(McpPathRule {
            id: "manual-extra".into(),
            prefix: "/other".into(),
            access: McpAccessMode::ReadOnly,
            source: McpRuleSource::Manual,
            confirmation_rules: None,
        });
        assert_eq!(
            validate_simple_workspace_exposure(&storage, &workspace)
                .unwrap_err()
                .code,
            McpErrorCode::ERR_CONFIRMATION_REQUIRED
        );
    }
}
