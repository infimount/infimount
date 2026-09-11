use infimount_core::workspaces::workspace_schema_supported;
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::policy::{McpAccessMode, McpRuleSource};
use infimount_mcp::registry::StorageRecord;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAgentAccessOutput {
    pub workspace_id: String,
    pub storage_id: String,
    pub access_profile: String,
    pub mcp_exposed: bool,
    pub changed: bool,
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
            && matches!(candidate.source, McpRuleSource::Manual)
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

    let workspace = state
        .workspaces
        .find_by_id(&workspaceId)
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to load workspace"))?
        .ok_or_else(|| err(McpErrorCode::ERR_INVALID_PATH, "workspace was not found"))?;
    let storage = state.find_storage_by_id(&workspace.storage_id)?;
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
            .ok_or_else(|| err(McpErrorCode::ERR_STORAGE_NOT_FOUND, "workspace storage was not found"))?;
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
        let root = std::env::temp_dir().join("infimount-agent-access-test");
        std::fs::create_dir_all(&root).unwrap();
        let mut storage = StorageRecord::new(
            "Local".into(),
            "local".into(),
            json!({ "root": root.to_string_lossy() }),
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
