use infimount_control::publication::validate_sha256;
pub use infimount_control::publication::{
    AgentTaskPublicationRequest, AgentTaskPublishConflictPolicy,
};
use infimount_control::publication_context::{validate_destination_path, validate_local_path};
use infimount_control::publication_engine::{
    build_publication_plan as shared_build_publication_plan, PublicationPlan,
};
use infimount_control::publication_plan::{
    AgentTaskPublishReceipt, AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
};
pub use infimount_control::publication_plan::{
    AgentTaskPublicationPlanFile, AgentTaskPublishAction,
};
use infimount_core::agent_task_io::{
    hash_agent_task_file, publish_reviewed_agent_task_file, AgentTaskFileDigest,
};
use infimount_core::CoreError;
use infimount_mcp::registry::StorageRecord;
use opendal::ErrorKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;
#[cfg(test)]
use infimount_control::publication::normalize_destination_dir;

const AGENT_TASK_PUBLISH_RECEIPT_PREFIX: &str = "publish-receipt-";

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationOutput {
    pub publication_id: String,
    pub published_at: String,
    pub workspace_id: String,
    pub task_id: String,
    pub destination_storage_id: String,
    pub destination_storage_name: String,
    pub receipt_path: String,
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
    if plan.preview_token != request.preview_token {
        return Err(CoreError::Config(
            "Agent Task publication changed since review; review the publication plan again".into(),
        ));
    }
    if plan.conflict_count != 0 {
        return Err(CoreError::Config(
            "Agent Task publication has unresolved destination conflicts".into(),
        ));
    }

    let mut published = Vec::<AgentTaskPublicationPlanFile>::new();
    for file in &plan.files {
        let source_path = join_path(&plan.context.workspace_task_path, &file.task_path);
        validate_local_path(&plan.context.workspace_storage, &source_path)?;
        validate_destination_path(&plan.context.destination_storage, &file.destination_path)?;
        ensure_publication_parent(
            &plan.context.destination_op,
            &plan.context.destination_storage,
            &file.destination_path,
        )
        .await?;

        let expected = AgentTaskFileDigest {
            byte_size: file.byte_size,
            sha256: file.sha256.clone(),
        };
        if let Err(error) = publish_reviewed_agent_task_file(
            &plan.context.workspace_op,
            &source_path,
            &plan.context.destination_op,
            &file.destination_path,
            &expected,
        )
        .await
        {
            if published.is_empty() {
                return Err(error);
            }
            // Some create-only destinations are already committed. OpenDAL 0.58
            // has no portable compare-and-delete primitive, so automatic rollback
            // could delete data replaced by another writer after our verification.
            return Err(CoreError::TransferCleanupRequired);
        }
        published.push(file.clone());
    }

    let publication_id = Uuid::new_v4().to_string();
    let published_at = chrono::Utc::now().to_rfc3339();
    let receipt = AgentTaskPublishReceipt {
        schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
        publication_id: publication_id.clone(),
        task_id: request.publication.task_id.clone(),
        workspace_id: request.publication.workspace_id.clone(),
        published_at: published_at.clone(),
        destination_storage_id: plan.context.destination_storage.id.clone(),
        destination_storage_name: plan.context.destination_storage.name.clone(),
        destination_dir: plan.context.destination_dir.clone(),
        conflict_policy: request.publication.conflict_policy,
        files: published.clone(),
    };
    let receipt_file = format!("{AGENT_TASK_PUBLISH_RECEIPT_PREFIX}{publication_id}.json");
    let receipt_path = join_path(&plan.context.workspace_task_path, &receipt_file);
    validate_local_path(&plan.context.workspace_storage, &receipt_path)?;
    let mut receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    receipt_bytes.push(b'\n');
    if write_publication_receipt(&plan.context.workspace_op, &receipt_path, &receipt_bytes)
        .await
        .is_err()
    {
        // Outputs have already committed. Preserve them and any ambiguous
        // receipt state for explicit cleanup rather than deleting by path.
        return Err(CoreError::TransferCleanupRequired);
    }

    Ok(AgentTaskPublicationOutput {
        publication_id,
        published_at,
        workspace_id: request.publication.workspace_id,
        task_id: request.publication.task_id,
        destination_storage_id: plan.context.destination_storage.id,
        destination_storage_name: plan.context.destination_storage.name,
        receipt_path: relative_to_root(&plan.context.workspace.root_path, &receipt_path)?,
        files: published,
    })
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

async fn ensure_publication_parent(
    op: &opendal::Operator,
    storage: &StorageRecord,
    destination_path: &str,
) -> Result<(), CoreError> {
    let Some((parent, _)) = destination_path.rsplit_once('/') else {
        return Ok(());
    };
    if parent.is_empty() || !op.info().capability().create_dir {
        return Ok(());
    }
    validate_destination_path(storage, parent)?;
    op.create_dir(&format!("{}/", parent.trim_end_matches('/')))
        .await?;
    Ok(())
}

async fn write_publication_receipt(
    op: &opendal::Operator,
    path: &str,
    bytes: &[u8],
) -> Result<(), CoreError> {
    if !op.info().capability().write_with_if_not_exists {
        return Err(CoreError::Config(
            "Agent Task workspace does not support safe create-only publication receipts".into(),
        ));
    }

    let expected = AgentTaskFileDigest {
        byte_size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    };
    let mut writer = op.writer_with(path).if_not_exists(true).await?;
    if let Err(error) = writer.write(bytes.to_vec()).await {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(error.into());
    }
    if let Err(error) = writer.close().await {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(error.into());
    }

    match hash_agent_task_file(op, path).await {
        Ok(actual) if actual == expected => Ok(()),
        Ok(_) => Err(CoreError::TransferCleanupRequired),
        Err(CoreError::Storage(error)) if error.kind() == ErrorKind::NotFound => {
            Err(CoreError::Config(
                "Agent Task publication receipt disappeared during integrity verification".into(),
            ))
        }
        Err(_) => Err(CoreError::TransferCleanupRequired),
    }
}

fn validate_preview_token(value: &str) -> Result<(), CoreError> {
    validate_sha256(value)
        .map_err(|_| CoreError::Config("invalid publication preview token".into()))
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

fn relative_to_root(root: &str, path: &str) -> Result<String, CoreError> {
    let root = root.trim().trim_matches('/');
    let path = path.trim().trim_matches('/');
    if root.is_empty() {
        return (!path.is_empty())
            .then(|| path.to_string())
            .ok_or_else(|| CoreError::Config("publication receipt path is invalid".into()));
    }
    path.strip_prefix(&format!("{root}/"))
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CoreError::Config("publication receipt escaped the workspace".into()))
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
