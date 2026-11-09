//! Private hosted-session lifecycle evidence and crash recovery (C2.9a).
//!
//! No HTTP transport, API-key lookup, exported IPC method or publication.
//! A future native coordinator must first verify user presence/consent,
//! configuration and workspace locks, and independently rehash input bytes.
//! This layer only makes ambiguous remote creates fail closed and persists
//! the provider session ID before any further event processing.
use infimount_core::agent_task_executions::{
    generate_execution_id, AdapterKind, AgentTaskExecutionRecord, ExecutionRecordStore,
    ExecutionStatus, IsolationProfile, RuntimeKind, RuntimeProvider, EXECUTION_SCHEMA_VERSION,
};
use infimount_core::agent_tasks::{AgentTaskBrief, AgentTaskManifest};
use infimount_core::CoreError;
use sha2::{Digest, Sha256};

use crate::sandbox_bundle::build_sandbox_task_bundle;

/// The provider create request may have been accepted even if its response
/// was lost. There is deliberately NO "retry create" recovery decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostedRecoveryAction {
    /// Nothing was sent by this private coordinator. Fresh approval still
    /// required before a new provider request may be issued.
    AwaitExplicitApproval,
    /// Running but the session ID was not captured. Inspect the provider
    /// account's sessions/logs. Never blindly create another paid session.
    ManualReconciliationRequired,
    /// Session ID was durably bound. Retrieve that same session and its
    /// saved items, then close/delete it as appropriate.
    RecoverPinnedSession(String),
    /// Terminal evidence is immutable. New execution is separately approved.
    Terminal,
}

fn fail() -> CoreError {
    CoreError::Config("private hosted execution lifecycle failed closed".into())
}

fn checked_model_id(value: &str) -> Result<(), CoreError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
    {
        return Err(fail());
    }
    Ok(())
}

fn assert_hosted_running(record: &AgentTaskExecutionRecord) -> Result<(), CoreError> {
    record.validate()?;
    if record.status != ExecutionStatus::Running
        || record.runtime_kind != RuntimeKind::RemoteSandbox
        || record.runtime_provider != Some(RuntimeProvider::Openai)
        || record.adapter_kind != AdapterKind::OpenaiSandbox
    {
        return Err(fail());
    }
    Ok(())
}

/// Persist a sanitized, declared-scope execution BEFORE any network call.
/// This does not grant authority to submit that network call.
pub(crate) fn prepare_hosted_execution(
    store: &ExecutionRecordStore,
    trusted_manifest: &AgentTaskManifest,
    separately_reviewed_brief: &AgentTaskBrief,
    model_id: &str,
    trusted_started_at: &str,
) -> Result<AgentTaskExecutionRecord, CoreError> {
    checked_model_id(model_id)?;
    let bundle = build_sandbox_task_bundle(trusted_manifest, separately_reviewed_brief)?;
    if bundle.requested_outputs.is_empty() {
        return Err(fail());
    }
    let scope_bytes = serde_json::to_vec(&(
        "infimount:v0.8.2:hosted-request-scope:v1",
        model_id,
        &bundle.instructions,
        &bundle.inputs,
        &bundle.requested_outputs,
    ))?;
    let record = AgentTaskExecutionRecord {
        schema_version: EXECUTION_SCHEMA_VERSION,
        execution_id: generate_execution_id(),
        task_id: trusted_manifest.task_id.clone(),
        workspace_id: trusted_manifest.workspace_id.clone(),
        adapter_kind: AdapterKind::OpenaiSandbox,
        adapter_version: "Agents API beta".into(),
        executable_sha256: None,
        runtime_kind: RuntimeKind::RemoteSandbox,
        runtime_provider: Some(RuntimeProvider::Openai),
        started_at: trusted_started_at.into(),
        finished_at: None,
        status: ExecutionStatus::Prepared,
        task_scope_digest: bundle.bundle_metadata_sha256,
        enabled_task_tools: Vec::new(),
        isolation_profile: IsolationProfile::RemoteBundleIsolated,
        prompt_sha256: format!("{:x}", Sha256::digest(scope_bytes)),
        output_file_count: 0,
        output_total_bytes: 0,
        output_aggregate_sha256: None,
        error_code: None,
        provider_execution_id: None,
    };
    // The existing private execution store handles bounded, locked,
    // create-only records and rejects overlapping active attempts.
    store.create(&record)?;
    Ok(record)
}

/// Commit Running BEFORE the one-shot paid session-create POST. If the
/// connection dies after this point, recovery MUST not create a new session.
pub(crate) fn mark_hosted_create_started(
    store: &ExecutionRecordStore,
    prepared: &AgentTaskExecutionRecord,
) -> Result<AgentTaskExecutionRecord, CoreError> {
    prepared.validate()?;
    if prepared.status != ExecutionStatus::Prepared
        || prepared.adapter_kind != AdapterKind::OpenaiSandbox
        || prepared.runtime_provider != Some(RuntimeProvider::Openai)
        || prepared.provider_execution_id.is_some()
    {
        return Err(fail());
    }
    let mut running = prepared.clone();
    running.status = ExecutionStatus::Running;
    store.transition(prepared, &running)?;
    Ok(running)
}

/// Called synchronously when the trusted parser first sees a provider-issued
/// session ID. Persist it under a cross-process CAS lock *before* processing
/// later SSE frames or attempting downloads. Returns updated evidence.
pub(crate) fn bind_hosted_session_once(
    store: &ExecutionRecordStore,
    running: &AgentTaskExecutionRecord,
    session_id: &str,
) -> Result<AgentTaskExecutionRecord, CoreError> {
    assert_hosted_running(running)?;
    if running.provider_execution_id.is_some()
        || session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(fail());
    }
    let mut bound = running.clone();
    bound.provider_execution_id = Some(session_id.into());
    store.transition(running, &bound)?;
    Ok(bound)
}

/// Restart decisions are based on the *durable* store, never SSE events or
/// a client-supplied "completed" boolean. This method does not issue HTTP.
pub(crate) fn recover_hosted_attempt(
    store: &ExecutionRecordStore,
    workspace_id: &str,
    task_id: &str,
    execution_id: &str,
) -> Result<HostedRecoveryAction, CoreError> {
    let record = store.read(workspace_id, task_id, execution_id)?;
    record.validate()?;
    if record.adapter_kind != AdapterKind::OpenaiSandbox
        || record.runtime_provider != Some(RuntimeProvider::Openai)
        || record.runtime_kind != RuntimeKind::RemoteSandbox
    {
        return Err(fail());
    }
    match record.status {
        ExecutionStatus::Prepared => Ok(HostedRecoveryAction::AwaitExplicitApproval),
        ExecutionStatus::Running => match record.provider_execution_id {
            None => Ok(HostedRecoveryAction::ManualReconciliationRequired),
            Some(session_id) => Ok(HostedRecoveryAction::RecoverPinnedSession(session_id)),
        },
        ExecutionStatus::Succeeded | ExecutionStatus::Failed | ExecutionStatus::Cancelled => {
            Ok(HostedRecoveryAction::Terminal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};

    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
    const WORKSPACE_ID: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";

    fn manifest() -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: TASK_ID.into(),
            workspace_id: WORKSPACE_ID.into(),
            task_root: format!("tasks/{TASK_ID}"),
            title: "PRIVATE original title".into(),
            created_at: "2026-11-10T00:00:00Z".into(),
            outputs_directory: "outputs".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/sample.csv".into(),
                byte_size: 5,
                sha256: format!("{:x}", Sha256::digest(b"alpha")),
            }],
        }
    }

    fn reviewed() -> AgentTaskBrief {
        AgentTaskBrief {
            title: "Reviewed CSV".into(),
            objective: "Report one synthetic statistic".into(),
            requested_outputs: vec!["outputs/report.md".into()],
        }
    }

    fn prepare(store: &ExecutionRecordStore) -> AgentTaskExecutionRecord {
        prepare_hosted_execution(
            store,
            &manifest(),
            &reviewed(),
            "gpt-6-astra",
            "2026-11-10T00:30:00Z",
        )
        .unwrap()
    }

    #[test]
    fn create_is_durable_single_active_and_private_scope_contains_no_source_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExecutionRecordStore::new(dir.path());
        let p = prepare(&store);
        assert_eq!(p.status, ExecutionStatus::Prepared);
        assert_eq!(p.provider_execution_id, None);
        assert_eq!(p.enabled_task_tools.len(), 0);
        assert_eq!(
            recover_hosted_attempt(&store, WORKSPACE_ID, TASK_ID, &p.execution_id).unwrap(),
            HostedRecoveryAction::AwaitExplicitApproval
        );
        assert!(prepare_hosted_execution(
            &store,
            &manifest(),
            &reviewed(),
            "gpt-6-astra",
            "2026-11-10T00:31:00Z"
        )
        .is_err());
        let serialized = serde_json::to_string(&p).unwrap();
        assert!(!serialized.contains("PRIVATE original title"));
        assert!(!serialized.contains("inputs/sample.csv"));
        assert!(!serialized.contains("Report one synthetic statistic"));
        let mut extra = reviewed();
        extra.requested_outputs.push("outputs/extra.md".into());
        let other = tempfile::tempdir().unwrap();
        let different = prepare_hosted_execution(
            &ExecutionRecordStore::new(other.path()),
            &manifest(),
            &extra,
            "gpt-6-astra",
            "2026-11-10T00:30:00Z",
        )
        .unwrap();
        assert_ne!(p.task_scope_digest, different.task_scope_digest);
    }

    #[test]
    fn unknown_session_after_create_must_be_manually_reconciled_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExecutionRecordStore::new(dir.path());
        let p = prepare(&store);
        let started = mark_hosted_create_started(&store, &p).unwrap();
        assert_eq!(
            recover_hosted_attempt(&store, WORKSPACE_ID, TASK_ID, &p.execution_id).unwrap(),
            HostedRecoveryAction::ManualReconciliationRequired
        );
        let restarted = ExecutionRecordStore::new(dir.path());
        assert_eq!(
            recover_hosted_attempt(&restarted, WORKSPACE_ID, TASK_ID, &p.execution_id).unwrap(),
            HostedRecoveryAction::ManualReconciliationRequired
        );
        assert!(mark_hosted_create_started(&restarted, &p).is_err());
        assert!(prepare_hosted_execution(
            &restarted,
            &manifest(),
            &reviewed(),
            "gpt-6-astra",
            "2026-11-10T00:32:00Z"
        )
        .is_err());
        assert_eq!(started.status, ExecutionStatus::Running);
    }

    #[test]
    fn first_streamed_session_id_is_durable_and_cannot_be_rebound() {
        let dir = tempfile::tempdir().unwrap();
        let first = ExecutionRecordStore::new(dir.path());
        let prepared = prepare(&first);
        let running = mark_hosted_create_started(&first, &prepared).unwrap();
        let bound = bind_hosted_session_once(&first, &running, "sess_real_1").unwrap();
        let second = ExecutionRecordStore::new(dir.path());
        assert_eq!(
            recover_hosted_attempt(&second, WORKSPACE_ID, TASK_ID, &bound.execution_id).unwrap(),
            HostedRecoveryAction::RecoverPinnedSession("sess_real_1".into())
        );
        assert!(bind_hosted_session_once(&second, &running, "sess_other").is_err());
        assert!(bind_hosted_session_once(&second, &bound, "sess_real_1").is_err());
        assert!(mark_hosted_create_started(&second, &prepared).is_err());
    }

    #[test]
    fn forged_credentials_or_unreviewed_scope_cannot_prepare_execution() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExecutionRecordStore::new(dir.path());
        let mut blank = reviewed();
        blank.requested_outputs.clear();
        assert!(prepare_hosted_execution(
            &store,
            &manifest(),
            &blank,
            "gpt-6-astra",
            "2026-11-10T00:30:00Z"
        )
        .is_err());
        assert!(prepare_hosted_execution(
            &store,
            &manifest(),
            &reviewed(),
            "https://attacker.invalid",
            "2026-11-10T00:30:00Z"
        )
        .is_err());
        let p = prepare(&store);
        let running = mark_hosted_create_started(&store, &p).unwrap();
        for bad in ["", "../secret", "sess id", &"x".repeat(129)] {
            assert!(bind_hosted_session_once(&store, &running, bad).is_err());
        }
        let mut foreign = running.clone();
        foreign.runtime_provider = Some(RuntimeProvider::Daytona);
        assert!(bind_hosted_session_once(&store, &foreign, "sess_1").is_err());
    }
}
