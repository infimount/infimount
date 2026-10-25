use infimount_control::publication::validate_sha256;
pub use infimount_control::publication::{
    AgentTaskPublicationRequest, AgentTaskPublishConflictPolicy,
};
use infimount_control::publication_apply::commit_locked_publication;
#[cfg(test)]
use infimount_control::publication_apply::write_publication_receipt;
pub use infimount_control::publication_apply::AgentTaskPublicationOutput;
use infimount_control::publication_engine::{
    build_publication_plan as shared_build_publication_plan, PublicationPlan,
};
pub use infimount_control::publication_plan::AgentTaskPublicationPlanFile;
#[cfg(test)]
use infimount_control::publication_plan::AgentTaskPublishAction;
#[cfg(test)]
use infimount_control::publication_plan::{
    AgentTaskPublishReceipt, AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
};
use infimount_core::CoreError;
#[cfg(test)]
use infimount_mcp::registry::StorageRecord;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;
#[cfg(test)]
use infimount_control::publication::normalize_destination_dir;
#[cfg(test)]
use infimount_control::publication_plan::{
    publication_preview_fingerprint, AgentTaskPublicationFingerprint,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyAgentTaskPublicationRequest {
    pub publication: AgentTaskPublicationRequest,
    pub preview_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationPreview {
    pub workspace_id: String,
    pub workspace_name: String,
    pub task_id: String,
    pub task_root: String,
    pub destination_storage_id: String,
    pub destination_storage_name: String,
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
    pub file_count: usize,
    pub total_bytes: u64,
    pub create_count: usize,
    pub rename_count: usize,
    pub conflict_count: usize,
    pub can_publish: bool,
    pub preview_token: String,
    pub files: Vec<AgentTaskPublicationPlanFile>,
}

#[tauri::command]
pub async fn preview_agent_task_publication(
    state: State<'_, AppState>,
    request: AgentTaskPublicationRequest,
) -> Result<AgentTaskPublicationPreview, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Agent Task publication is unavailable while Infimount is degraded".into(),
        )
    })?;
    let plan = build_publication_plan(&state, &request).await?;
    let context = &plan.context;

    Ok(AgentTaskPublicationPreview {
        workspace_id: context.workspace.id.clone(),
        workspace_name: context.workspace.name.clone(),
        task_id: request.task_id,
        task_root: context.task_root.clone(),
        destination_storage_id: context.destination_storage.id.clone(),
        destination_storage_name: context.destination_storage.name.clone(),
        destination_dir: context.destination_dir.clone(),
        conflict_policy: request.conflict_policy,
        file_count: plan.files.len(),
        total_bytes: plan.total_bytes,
        create_count: plan.create_count,
        rename_count: plan.rename_count,
        conflict_count: plan.conflict_count,
        can_publish: plan.conflict_count == 0,
        preview_token: plan.preview_token,
        files: plan.files,
    })
}

#[tauri::command]
pub async fn publish_agent_task_outputs(
    state: State<'_, AppState>,
    request: ApplyAgentTaskPublicationRequest,
) -> Result<AgentTaskPublicationOutput, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Agent Task publication is unavailable while Infimount is degraded".into(),
        )
    })?;
    validate_preview_token(&request.preview_token)?;

    // Freeze storage configuration for the complete apply transaction. This
    // follows the repository's existing mutation lock order and prevents either
    // the task workspace or destination storage from being repointed after the
    // approved plan is rebuilt but before its bytes and receipt are committed.
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _config_transaction = state
        .registry
        .acquire_configuration_transaction()
        .map_err(crate::state::mcp_error_to_core_error)?;
    state
        .recover_and_require_clean_configuration_locked()
        .map_err(crate::state::mcp_error_to_core_error)?;

    // Serialize workspace create/update/delete against the complete publication
    // transaction. File browsing and ordinary storage reads remain unaffected.
    let _workspace_transaction = state.workspaces.acquire_mutation_lock()?;

    // Rebuild the complete plan from current source bytes and current destination
    // state. The preview token is a staleness token, not an authorization token.
    let plan = build_publication_plan(&state, &request.publication).await?;
    commit_locked_publication(&plan, &request.publication, &request.preview_token, || {
        (Uuid::new_v4().to_string(), chrono::Utc::now().to_rfc3339())
    })
    .await
}

async fn build_publication_plan(
    state: &AppState,
    request: &AgentTaskPublicationRequest,
) -> Result<PublicationPlan, CoreError> {
    shared_build_publication_plan(
        request,
        |id| state.workspaces.find_by_id(id),
        |id| {
            state
                .find_storage_by_id(id)
                .map_err(crate::state::mcp_error_to_core_error)
        },
        |id| state.operator_for_storage_id(id),
    )
    .await
}

fn validate_preview_token(value: &str) -> Result<(), CoreError> {
    validate_sha256(value)
        .map_err(|_| CoreError::Config("invalid publication preview token".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_control::publication::validate_output_task_path;
    use infimount_control::publication_engine::{
        destination_aliases_reserved_workspace_control_path, is_reserved_workspace_destination,
    };

    #[test]
    fn destination_paths_reject_traversal_and_windows_separators() {
        assert_eq!(
            normalize_destination_dir("/exports/reports/").unwrap(),
            "exports/reports"
        );
        assert!(normalize_destination_dir("../secret").is_err());
        assert!(normalize_destination_dir("exports\\secret").is_err());
        assert!(validate_output_task_path("outputs/report.md").is_ok());
        assert!(validate_output_task_path("inputs/report.md").is_err());
    }

    #[test]
    fn reserved_workspace_control_paths_are_not_publication_destinations() {
        let workspace = "/agent-workspaces/coding";
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/tasks/new-task/report.md"
        ));
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/TASKS/new-task/report.md"
        ));
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/.INFIMOUNT/checkpoints/fake.json"
        ));
        assert!(!is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/published/report.md"
        ));
        assert!(!is_reserved_workspace_destination(
            workspace,
            "elsewhere/tasks/report.md"
        ));
    }

    #[test]
    fn reserved_control_paths_reject_nested_local_storage_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let storage_root = temp.path().join("storage");
        let workspace_root = "agent-workspaces/coding";
        let tasks_root = storage_root.join(workspace_root).join("tasks");
        let internal_root = storage_root.join(workspace_root).join(".infimount");
        let other_root = temp.path().join("other");
        std::fs::create_dir_all(&tasks_root).unwrap();
        std::fs::create_dir_all(&internal_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();

        let workspace_storage = StorageRecord::new(
            "workspace".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": storage_root.to_string_lossy() }),
        );
        let tasks_alias = StorageRecord::new(
            "tasks-alias".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": tasks_root.to_string_lossy() }),
        );
        let internal_alias = StorageRecord::new(
            "internal-alias".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": internal_root.to_string_lossy() }),
        );
        let unrelated = StorageRecord::new(
            "unrelated".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": other_root.to_string_lossy() }),
        );

        assert!(destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &tasks_alias,
            "published/report.md",
        )
        .unwrap());
        assert!(destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &internal_alias,
            "checkpoints/fake.json",
        )
        .unwrap());
        assert!(!destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &unrelated,
            "published/report.md",
        )
        .unwrap());
    }

    #[tokio::test]
    async fn publication_receipts_are_create_only_and_integrity_checked() {
        let op = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
        assert!(op.info().capability().write_with_if_not_exists);
        let path = "tasks/task/publish-receipt-publication.json";
        let bytes = b"{\"publicationId\":\"publication\"}\n";

        write_publication_receipt(&op, path, bytes).await.unwrap();
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
        assert!(write_publication_receipt(&op, path, bytes).await.is_err());
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
    }

    #[test]
    fn publication_receipt_does_not_contain_source_storage_or_host_paths() {
        let receipt = AgentTaskPublishReceipt {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            publication_id: "publication-id".into(),
            task_id: "task-id".into(),
            workspace_id: "workspace-id".into(),
            published_at: "2026-09-08T00:00:00Z".into(),
            destination_storage_id: "destination-id".into(),
            destination_storage_name: "Published".into(),
            destination_dir: "exports".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Fail,
            files: vec![AgentTaskPublicationPlanFile {
                task_path: "outputs/report.md".into(),
                byte_size: 3,
                sha256: "a".repeat(64),
                destination_path: "exports/report.md".into(),
                action: AgentTaskPublishAction::Create,
            }],
        };
        let json = serde_json::to_value(receipt).unwrap();
        assert!(json.get("sourceStorageId").is_none());
        assert!(json.get("sourcePath").is_none());
        assert!(json.get("hostPath").is_none());
    }

    #[test]
    fn preview_token_binds_destination_and_reviewed_hashes() {
        let files = vec![AgentTaskPublicationPlanFile {
            task_path: "outputs/report.md".into(),
            byte_size: 3,
            sha256: "a".repeat(64),
            destination_path: "exports/report.md".into(),
            action: AgentTaskPublishAction::Create,
        }];
        let fingerprint = AgentTaskPublicationFingerprint {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            workspace_id: "workspace",
            task_id: "task",
            workspace_namespace: "workspace-ns",
            destination_storage_id: "destination",
            destination_namespace: "destination-ns",
            destination_dir: "exports",
            conflict_policy: &AgentTaskPublishConflictPolicy::Fail,
            files: &files,
        };
        let token = publication_preview_fingerprint(&fingerprint).unwrap();
        let mut changed = files.clone();
        changed[0].sha256 = "b".repeat(64);
        let changed_fingerprint = AgentTaskPublicationFingerprint {
            files: &changed,
            ..fingerprint
        };
        assert_ne!(
            token,
            publication_preview_fingerprint(&changed_fingerprint).unwrap()
        );
    }
}
