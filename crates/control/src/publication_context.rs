//! Trusted, fail-closed publication context binding shared by desktop and
//! future local control entrypoints. Does not acquire mutation locks.
//!
//! The caller must hold the authoritative configuration/workspace locks for
//! apply, and must re-resolve this context before any create-only write.
//! Supplying a preview token or calling this resolver never grants approval.
use infimount_core::agent_tasks::agent_task_root;
use infimount_core::workspaces::{workspace_schema_supported, WorkspaceRecord};
use infimount_core::{CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use infimount_mcp::storage_namespace::{storage_namespace_fingerprint, validate_local_mcp_path};
use opendal::Operator;

use crate::publication::{
    normalize_destination_dir, AgentTaskPublicationRequest, MAX_AGENT_TASK_DESTINATION_PATH_BYTES,
};

pub struct PublicationContext {
    pub workspace: WorkspaceRecord,
    pub workspace_storage: StorageRecord,
    pub destination_storage: StorageRecord,
    pub workspace_op: Operator,
    pub destination_op: Operator,
    pub workspace_namespace: String,
    pub destination_namespace: String,
    pub task_root: String,
    pub workspace_task_path: String,
    pub destination_dir: String,
}

/// Resolve a task's workspace and both storage operators in the original
/// fail-closed ordering. Lookups must be backed by trusted local registries;
/// agents are never supplied these resolver callbacks.
///
/// Apply callers MUST retain existing lifecycle/configuration/workspace locks
/// across resolution, replan, byte verification, writes and receipt persistence.
/// Preview callers resolve without a mutation lock and gain no write authority.
pub fn resolve_publication_context<FindWorkspace, FindStorage, OpenStorage>(
    request: &AgentTaskPublicationRequest,
    find_workspace: FindWorkspace,
    mut find_storage: FindStorage,
    mut open_storage: OpenStorage,
) -> Result<PublicationContext, CoreError>
where
    FindWorkspace: FnOnce(&str) -> Result<Option<WorkspaceRecord>, CoreError>,
    FindStorage: FnMut(&str) -> Result<StorageRecord, CoreError>,
    OpenStorage: FnMut(&str) -> Result<Operator, CoreError>,
{
    let workspace = find_workspace(&request.workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }
    if workspace.access_profile != "read_write" {
        return Err(CoreError::Config(
            "Agent Task publication requires the task's read-write Agent Workspace".into(),
        ));
    }

    let workspace_storage = find_storage(&workspace.storage_id).map_err(|_| {
        CoreError::Config("Agent Task workspace storage could not be validated".into())
    })?;
    if !workspace_storage.enabled || workspace_storage.read_only {
        return Err(CoreError::Config(
            "Agent Task workspace storage must remain enabled and writable for publication".into(),
        ));
    }
    let workspace_kind = workspace_storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(workspace_kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Task publication currently requires a Local Filesystem task workspace".into(),
        ));
    }
    let workspace_namespace = storage_namespace_fingerprint(&workspace_storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".into()))?;
    if workspace_namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let destination_storage = find_storage(&request.destination_storage_id).map_err(|_| {
        CoreError::Config("publication destination storage could not be found".into())
    })?;
    if !destination_storage.enabled || destination_storage.read_only {
        return Err(CoreError::Config(
            "publication destination storage must be enabled and writable".into(),
        ));
    }
    let destination_namespace =
        storage_namespace_fingerprint(&destination_storage).map_err(|_| {
            CoreError::Config("failed to verify publication destination identity".into())
        })?;

    let workspace_op = open_storage(&workspace_storage.id)?;
    let workspace_caps = workspace_op.info().capability();
    if !workspace_caps.stat
        || !workspace_caps.read
        || !workspace_caps.write
        || !workspace_caps.write_with_if_not_exists
    {
        return Err(CoreError::Config(
            "Agent Task workspace lacks required safe receipt capabilities for publication".into(),
        ));
    }

    let destination_op = open_storage(&destination_storage.id)?;
    let destination_caps = destination_op.info().capability();
    if !destination_caps.stat
        || !destination_caps.read
        || !destination_caps.write
        || !destination_caps.write_with_if_not_exists
    {
        return Err(CoreError::Config(
            "publication destination does not support safe create-only publication".into(),
        ));
    }

    let task_root = agent_task_root(&request.task_id)?;
    let workspace_task_path = join_path(&workspace.root_path, &task_root);
    validate_local_path(&workspace_storage, &workspace.root_path)?;
    validate_local_path(&workspace_storage, &workspace_task_path)?;
    let destination_dir = normalize_destination_dir(&request.destination_dir)?;
    validate_destination_path(&destination_storage, &destination_dir)?;

    Ok(PublicationContext {
        workspace,
        workspace_storage,
        destination_storage,
        workspace_op,
        destination_op,
        workspace_namespace,
        destination_namespace,
        task_root,
        workspace_task_path,
        destination_dir,
    })
}

pub fn validate_local_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    validate_local_mcp_path(storage, path)
        .map_err(|_| CoreError::Config("Agent Task local path confinement check failed".into()))
}

pub fn validate_destination_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    let normalized = path.trim_matches('/');
    if normalized.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES
        || normalized.contains('\\')
        || normalized.chars().any(|ch| ch == '\0' || ch.is_control())
        || (!normalized.is_empty()
            && normalized
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == ".."))
    {
        return Err(CoreError::Config(
            "publication destination path is invalid or exceeds the path length limit".into(),
        ));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("publication destination backend is unsupported".into()))?;
    if matches!(kind, SourceKind::Local) && !normalized.is_empty() {
        validate_local_path(storage, normalized)?;
    }
    Ok(())
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
    use infimount_core::workspaces::WORKSPACE_RECORD_SCHEMA_VERSION;
    use serde_json::json;

    const WORKSPACE_ID: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";
    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn request() -> AgentTaskPublicationRequest {
        AgentTaskPublicationRequest {
            workspace_id: WORKSPACE_ID.into(),
            task_id: TASK_ID.into(),
            outputs: Vec::new(),
            destination_storage_id: "destination".into(),
            destination_dir: "exports".into(),
            conflict_policy: crate::publication::AgentTaskPublishConflictPolicy::Fail,
        }
    }

    fn workspace(storage: &StorageRecord) -> WorkspaceRecord {
        WorkspaceRecord {
            id: WORKSPACE_ID.into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Coding".into(),
            root_path: "workspaces/coding".into(),
            template_id: "coding".into(),
            access_profile: "read_write".into(),
            policy_rule_id: None,
            storage_namespace_fingerprint: storage_namespace_fingerprint(storage).unwrap(),
            created_at: String::new(),
            updated_at: String::new(),
            memory_files: vec![],
            checkpoint_ids: vec![],
        }
    }

    fn source_storage(root: &str) -> StorageRecord {
        let mut record = StorageRecord::new("Source".into(), "local".into(), json!({"root": root}));
        record.id = "source".into();
        record
    }

    fn destination_storage(root: &str) -> StorageRecord {
        let mut record =
            StorageRecord::new("Destination".into(), "local".into(), json!({"root": root}));
        record.id = "destination".into();
        record
    }

    #[test]
    fn refuses_unsupported_workspace_schema_before_storage_resolution() {
        let folder = tempfile::tempdir().unwrap();
        let storage = source_storage(&folder.path().to_string_lossy());
        let mut workspace = workspace(&storage);
        workspace.schema_version = 0;
        let result = resolve_publication_context(
            &request(),
            |_| Ok(Some(workspace)),
            |_| panic!("must reject workspace before storage lookup"),
            |_| panic!("must not open a storage operator"),
        );
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("unsupported schema"));
    }

    #[test]
    fn refuses_stale_workspace_namespace_before_destination_lookup() {
        let folder = tempfile::tempdir().unwrap();
        let storage = source_storage(&folder.path().to_string_lossy());
        let mut workspace = workspace(&storage);
        workspace.storage_namespace_fingerprint = "stale".into();
        let result = resolve_publication_context(
            &request(),
            |_| Ok(Some(workspace)),
            |_| Ok(storage.clone()),
            |_| panic!("must not open a storage operator"),
        );
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("identity changed"));
    }

    #[test]
    fn refuses_disabled_destination_before_opening_storage_operators() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_string_lossy();
        let source = source_storage(&root);
        let mut destination = destination_storage(&root);
        destination.enabled = false;
        let context = workspace(&source);
        let result = resolve_publication_context(
            &request(),
            |_| Ok(Some(context)),
            |id| {
                if id == "source" {
                    Ok(source.clone())
                } else {
                    Ok(destination.clone())
                }
            },
            |_| panic!("must not open a storage operator"),
        );
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("enabled and writable"));
    }

    #[test]
    fn destination_path_guards_reject_escapes_and_backslash_paths() {
        let folder = tempfile::tempdir().unwrap();
        let storage = destination_storage(&folder.path().to_string_lossy());
        assert!(validate_destination_path(&storage, "exports/../secret").is_err());
        assert!(validate_destination_path(&storage, "exports\\secret").is_err());
        assert!(validate_destination_path(
            &storage,
            &"x".repeat(MAX_AGENT_TASK_DESTINATION_PATH_BYTES + 1)
        )
        .is_err());
    }

    #[test]
    fn valid_workspace_and_storage_are_resolved_without_any_mutation() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_string_lossy().to_string();
        let source = source_storage(&root);
        let destination = destination_storage(&root);
        let ws = workspace(&source);
        let op = Operator::new(opendal::services::Fs::default().root(&root)).unwrap();
        let result = resolve_publication_context(
            &request(),
            |_| Ok(Some(ws)),
            |id| {
                if id == "source" {
                    Ok(source.clone())
                } else {
                    Ok(destination.clone())
                }
            },
            |_| Ok(op.clone()),
        )
        .unwrap();
        assert_eq!(result.workspace.id, WORKSPACE_ID);
        assert_eq!(result.destination_dir, "exports");
        assert_eq!(result.task_root, format!("tasks/{TASK_ID}"));
        assert!(
            result
                .workspace_op
                .info()
                .capability()
                .write_with_if_not_exists
        );
    }
}
