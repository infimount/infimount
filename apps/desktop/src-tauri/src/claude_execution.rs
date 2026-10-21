//! Trusted, internal orchestration of one Claude Code Agent Task execution.
//! Not exported as a Tauri command until packaged real-client qualification.
//! A2 evidence describes process completion plus independently hashed output
//! bytes. Review and publication remain separate trusted user decisions.
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use infimount_core::agent_task_executions::{
    generate_execution_id, AdapterKind, AgentTaskExecutionRecord, ExecutionStatus,
    IsolationProfile, RuntimeKind, TaskTool, EXECUTION_SCHEMA_VERSION,
};
use infimount_core::agent_task_io::hash_agent_task_file;
use infimount_core::{operations, CoreError};
use infimount_mcp::AgentTaskScope;
use sha2::{Digest, Sha256};

use crate::agent_task_handoff::{
    agent_task_prompt, cleanup_neutral_directory, create_neutral_handoff_directory,
    prepare_agent_task_handoff,
};
use crate::claude_handoff::claude_argv;
use crate::claude_runner::{detect_claude_at, execute_verified_claude, ClaudeOutcome};
use crate::state::AppState;

const MAX_OUTPUT_FILES: usize = 1_000;
const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RUN_TIMEOUT: Duration = Duration::from_secs(12 * 60);

struct NeutralDirectory(std::path::PathBuf);

impl Drop for NeutralDirectory {
    fn drop(&mut self) {
        cleanup_neutral_directory(&self.0);
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn scope_digest(scope: &AgentTaskScope) -> Result<String, CoreError> {
    // Bind execution evidence to the exact task grant without retaining
    // original source paths or host config contents in the private record.
    let contract = serde_json::json!({
        "taskId": scope.task_id,
        "workspaceId": scope.workspace_id,
        "storageId": scope.storage_id,
        "policyRuleId": scope.policy_rule_id,
        "namespaceSha256": scope.storage_namespace_fingerprint,
        "workspacePrefix": scope.workspace_prefix,
        "taskPrefix": scope.task_prefix,
        "outputsPrefix": scope.outputs_prefix,
        "tools": ["list_dir", "stat_path", "read_file", "search_paths", "write_file", "mkdir"],
    });
    Ok(sha256(&serde_json::to_vec(&contract)?))
}

fn new_record(
    scope: &AgentTaskScope,
    version: &str,
    executable_sha: &str,
    prompt: &str,
) -> Result<AgentTaskExecutionRecord, CoreError> {
    let record = AgentTaskExecutionRecord {
        schema_version: EXECUTION_SCHEMA_VERSION,
        execution_id: generate_execution_id(),
        task_id: scope.task_id.clone(),
        workspace_id: scope.workspace_id.clone(),
        adapter_kind: AdapterKind::ClaudeCode,
        adapter_version: version.to_owned(),
        executable_sha256: Some(executable_sha.to_owned()),
        runtime_kind: RuntimeKind::LocalClient,
        runtime_provider: None,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
        status: ExecutionStatus::Prepared,
        task_scope_digest: scope_digest(scope)?,
        enabled_task_tools: vec![
            TaskTool::ListDir, TaskTool::StatPath, TaskTool::ReadFile,
            TaskTool::SearchPaths, TaskTool::WriteFile, TaskTool::Mkdir,
        ],
        isolation_profile: IsolationProfile::McpOnlyReadonlyHost,
        prompt_sha256: sha256(prompt.as_bytes()),
        output_file_count: 0,
        output_total_bytes: 0,
        output_aggregate_sha256: None,
        error_code: None,
        provider_execution_id: None,
    };
    record.validate()?;
    Ok(record)
}

#[derive(Debug)]
struct OutputProof {
    count: u32,
    bytes: u64,
    sha256: Option<String>,
}

/// Enumerates bounded outputs using the same OpenDAL and host-path safety
/// checks as the desktop review. The evidence is a snapshot, not approval to
/// publish. A later independent user review must rehash the files again.
async fn output_proof(
    state: &AppState,
    scope: &AgentTaskScope,
) -> Result<OutputProof, CoreError> {
    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| CoreError::Config("Agent Task MCP scope changed during execution".into()))?;
    let op = state.operator_for_storage_id(&scope.storage_id)?;
    let mut entries = operations::list_entries_recursive(&op, &scope.outputs_prefix).await?;
    if entries.len() >= infimount_core::models::MAX_RECURSIVE_ITEMS as usize {
        return Err(CoreError::Config("Agent Task output set is truncated".into()));
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let storage = state.find_storage_by_id(&scope.storage_id)?;
    let mut count = 0usize;
    let mut total = 0u64;
    let mut combined = Sha256::new();

    for entry in entries {
        // Includes directory entries: a symlink or Windows reparse point cannot
        // be traversed on the way to a generated output.
        infimount_mcp::storage_namespace::validate_local_mcp_path(&storage, &entry.path)
            .map_err(|_| CoreError::Config("Agent Task output path changed or escaped".into()))?;
        if entry.is_dir {
            continue;
        }
        if !entry.path.starts_with(&format!("{}/", scope.outputs_prefix.trim_end_matches('/'))) {
            return Err(CoreError::Config("Agent Task output escaped the task scope".into()));
        }
        count += 1;
        if count > MAX_OUTPUT_FILES {
            return Err(CoreError::Config("Agent Task generated too many output files".into()));
        }
        let digest = hash_agent_task_file(&op, &entry.path).await?;
        total = total.checked_add(digest.byte_size)
            .ok_or_else(|| CoreError::Config("Agent Task output byte count overflowed".into()))?;
        if total > MAX_OUTPUT_BYTES {
            return Err(CoreError::Config("Agent Task generated outputs exceed the byte limit".into()));
        }
        combined.update(entry.path.as_bytes());
        combined.update([0_u8]);
        combined.update(digest.byte_size.to_be_bytes());
        combined.update(digest.sha256.as_bytes());
        combined.update([0_u8]);
    }

    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| CoreError::Config("Agent Task scope changed after execution".into()))?;
    Ok(OutputProof {
        count: count as u32,
        bytes: total,
        sha256: (count > 0).then(|| format!("{:x}", combined.finalize())),
    })
}

fn terminal_record(
    running: &AgentTaskExecutionRecord,
    outcome: ClaudeOutcome,
    proof: Option<&OutputProof>,
) -> AgentTaskExecutionRecord {
    let mut record = running.clone();
    record.finished_at = Some(Utc::now().to_rfc3339());
    if let Some(proof) = proof {
        record.output_file_count = proof.count;
        record.output_total_bytes = proof.bytes;
        record.output_aggregate_sha256.clone_from(&proof.sha256);
    }
    let successful_with_outputs = outcome == ClaudeOutcome::Succeeded
        && proof.is_some_and(|p| p.count > 0);
    record.status = if successful_with_outputs {
        ExecutionStatus::Succeeded
    } else if outcome == ClaudeOutcome::Cancelled {
        ExecutionStatus::Cancelled
    } else {
        ExecutionStatus::Failed
    };
    record.error_code = if successful_with_outputs {
        None
    } else if outcome == ClaudeOutcome::Succeeded {
        Some("ERR_CLAUDE_NO_OUTPUT".into())
    } else {
        Some(outcome.code().unwrap_or("ERR_CLAUDE_EXECUTION_FAILED").into())
    };
    record
}

/// Internal integration entry point for a future qualified adapter.
///
/// The explicit executable comes from the trusted local adapter configuration,
/// never from agent content. There is deliberately no Tauri registration or
/// frontend button yet. Windows execution refuses until job-object containment
/// is proven; other platforms still require a real authenticated packaged pilot.
pub(crate) async fn execute_claude_task(
    state: &AppState,
    executable: &Path,
    workspace_id: &str,
    task_id: &str,
    cancellation: Arc<AtomicBool>,
) -> Result<String, CoreError> {
    let prepared = prepare_agent_task_handoff(state, "Claude", workspace_id, task_id).await?;
    let sidecar = crate::activation_probe::verified_sidecar_path()
        .map_err(|_| CoreError::Config("Verified Infimount MCP sidecar is unavailable".into()))?;
    let prompt = agent_task_prompt(
        &prepared.workspace_name,
        &prepared.mcp_task_path,
        &prepared.mcp_outputs_path,
    );

    // A new attempt cannot silently overwrite or reuse reviewable outputs.
    let existing = output_proof(state, &prepared.scope).await?;
    if existing.count != 0 {
        return Err(CoreError::Config(
            "Agent Task already has generated output files; prepare a new task".into(),
        ));
    }

    let path = executable.to_path_buf();
    let identity = tauri::async_runtime::spawn_blocking(move || detect_claude_at(&path))
        .await
        .map_err(|_| CoreError::Config("Claude Code capability check failed".into()))??;
    let args = claude_argv(
        &sidecar,
        &prepared.scope,
        &prompt,
        &identity.reported_version,
    )?;
    let neutral = NeutralDirectory(create_neutral_handoff_directory("Claude")?);
    let store = state.execution_records.clone();
    let initial = new_record(
        &prepared.scope,
        &identity.reported_version,
        &identity.sha256,
        &prompt,
    )?;
    store.create(&initial)?;
    let mut running = initial.clone();
    running.status = ExecutionStatus::Running;
    store.transition(&initial, &running)?;

    let cwd = neutral.0.clone();
    let attempt = tauri::async_runtime::spawn_blocking(move || {
        execute_verified_claude(&identity, &args, &cwd, &cancellation, RUN_TIMEOUT)
    }).await;
    let outcome = match attempt {
        Ok(Ok(report)) => report.result,
        _ => ClaudeOutcome::Failed,
    };
    let proof = output_proof(state, &prepared.scope).await.ok();
    let completed = terminal_record(&running, outcome, proof.as_ref());
    // Failing to persist the terminal record never becomes a claimed success.
    store.transition(&running, &completed)?;
    Ok(initial.execution_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scope() -> AgentTaskScope {
        AgentTaskScope::new(
            "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            "storage-id".into(),
            "Workspace storage".into(),
            "workspace-id".into(),
            "workspace:workspace-id".into(),
            "namespace-fingerprint".into(),
            "agent/research".into(),
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454/outputs".into(),
        ).unwrap()
    }

    #[test]
    fn evidence_records_contain_only_hashes_and_task_ids() {
        let scope = test_scope();
        let r = new_record(
            &scope,
            "2.1.259 (Claude Code)",
            &"a".repeat(64),
            "Use task-only MCP",
        ).unwrap();
        let serialized = serde_json::to_string(&r).unwrap();
        assert_eq!(r.status, ExecutionStatus::Prepared);
        assert_eq!(r.enabled_task_tools.len(), 6);
        assert!(!serialized.contains("hostPath"));
        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("Bearer "));
        assert_eq!(r.task_scope_digest.len(), 64);
    }

    #[test]
    fn successful_cli_exit_without_reviewable_outputs_is_not_success() {
        let mut r = new_record(&test_scope(), "2.1.259", &"b".repeat(64), "task").unwrap();
        r.status = ExecutionStatus::Running;
        let finished = terminal_record(&r, ClaudeOutcome::Succeeded, None);
        assert_eq!(finished.status, ExecutionStatus::Failed);
        assert_eq!(finished.error_code.as_deref(), Some("ERR_CLAUDE_NO_OUTPUT"));
        let proof = OutputProof { count: 1, bytes: 4, sha256: Some("c".repeat(64)) };
        let finished = terminal_record(&r, ClaudeOutcome::Succeeded, Some(&proof));
        assert_eq!(finished.status, ExecutionStatus::Succeeded);
        assert_eq!(finished.output_file_count, 1);
        finished.validate().unwrap();
    }

    #[test]
    fn cancelled_or_timedout_process_never_sets_success() {
        let mut r = new_record(&test_scope(), "2.1.259", &"b".repeat(64), "task").unwrap();
        r.status = ExecutionStatus::Running;
        for outcome in [ClaudeOutcome::Cancelled, ClaudeOutcome::TimedOut, ClaudeOutcome::Failed] {
            let terminal = terminal_record(&r, outcome, None);
            assert_ne!(terminal.status, ExecutionStatus::Succeeded);
            assert!(terminal.error_code.is_some());
            terminal.validate().unwrap();
        }
    }
}
