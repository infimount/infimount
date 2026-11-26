//! Private Linux lifecycle connecting real confined files to a MOCKED provider.
//!
//! There is no HTTP adapter, API-key lookup, IPC command or production consent
//! constructor. The test-only consent fixture is NOT native user presence.
//! A future trusted caller must hold lifecycle/configuration/workspace locks
//! and qualify a real native consent issuer and bounded authenticated transport.
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

use infimount_core::agent_task_executions::{
    AgentTaskExecutionRecord, ExecutionRecordStore, ExecutionStatus,
};
use infimount_core::agent_tasks::{agent_task_root, AgentTaskBrief, AgentTaskManifest};
use infimount_core::CoreError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::hosted_cleanup_evidence::persist_public_deletion;
use crate::hosted_execution_recovery::{
    bind_hosted_session_once, mark_hosted_create_started, prepare_hosted_execution,
};
use crate::openai_hosted_artifacts::{
    validate_downloaded_hosted_artifacts, DownloadedHostedArtifact, HostedArtifactInventory,
    MAX_HOSTED_ARTIFACT_PAGES,
};
use crate::openai_hosted_events::{HostedSessionEventGate, HostedSessionObservedOutcome};
use crate::openai_hosted_items::{HostedSavedItemAudit, MAX_HOSTED_ITEM_PAGES};
use crate::openai_hosted_request::OPENAI_HOSTED_REQUEST_MAX_JSON_BYTES;
use crate::sandbox_bundle::build_sandbox_task_bundle;
use crate::sandbox_import::{
    validate_sandbox_output_listing, SandboxOutputCandidatePlan, SandboxRemoteEntryKind,
    SandboxRemoteOutputEntry, SandboxRemoteOutputListing, SANDBOX_IMPORT_SCHEMA_VERSION,
};
use crate::sandbox_local_export::build_verified_hosted_request_from_pinned_task;
use crate::sandbox_local_ingest::{
    ingest_reviewed_outputs_to_pinned_directory, open_absolute_root,
    require_empty_pinned_output_directory, require_existing_chain, VerifiedStagedOutput,
};

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 10_000;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_OUTPUT_FILES: usize = 32;
const MAX_CONSENT_LIFETIME_MS: u64 = 120_000;
const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);

fn fail(code: &'static str) -> CoreError {
    CoreError::Config(code.into())
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn check_deadline(deadline: Instant) -> Result<(), CoreError> {
    if Instant::now() >= deadline {
        return Err(fail("ERR_HOSTED_DEADLINE"));
    }
    Ok(())
}

/// Only trusted native state may supply this context. No user-chosen OS path.
pub(crate) struct HostedRunContext<'a> {
    pub workspace_root: &'a Path,
    pub manifest: &'a AgentTaskManifest,
    pub reviewed_brief: &'a AgentTaskBrief,
    pub model: &'a str,
    pub storage_namespace: &'a str,
    pub storage_revision: u64,
    pub records: &'a ExecutionRecordStore,
}

/// Native clock, never timestamps received from the model or provider.
pub(crate) trait HostedClock {
    fn unix_ms(&self) -> u64;
    fn rfc3339(&self) -> String;
}

/// Not Clone, Deserialize or Serialize. Ownership consumes a claim even on
/// mismatch. No production constructor exists, and this cannot authorize
/// publication. A future issuer must verify the exact native review first.
pub(crate) struct HostedExportConsent {
    snapshot_digest: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
}

impl HostedExportConsent {
    fn consume(self, snapshot_digest: &str, now: u64) -> Result<(), CoreError> {
        if self.snapshot_digest != snapshot_digest
            || self.issued_at_ms > now
            || now >= self.expires_at_ms
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms > MAX_CONSENT_LIFETIME_MS
        {
            return Err(fail("ERR_HOSTED_CONSENT"));
        }
        Ok(())
    }

    #[cfg(test)]
    fn fixture(snapshot_digest: String, now: u64) -> Self {
        Self {
            snapshot_digest,
            issued_at_ms: now,
            expires_at_ms: now + MAX_CONSENT_LIFETIME_MS,
        }
    }
}

/// Typed resources prevent a transport caller from supplying arbitrary URLs.
/// There is intentionally no production implementation of this trait.
pub(crate) enum HostedRead<'a> {
    Session {
        session_id: &'a str,
    },
    SavedItems {
        session_id: &'a str,
        turn_id: &'a str,
        after: Option<&'a str>,
    },
    Artifacts {
        session_id: &'a str,
        after: Option<&'a str>,
    },
    ArtifactBytes {
        session_id: &'a str,
        artifact_id: &'a str,
    },
}

pub(crate) type Receive<'a> = dyn FnMut(&[u8]) -> Result<(), CoreError> + 'a;

/// Every implementation must stop on callback errors, enforce its deadline
/// during blocking I/O and bind authenticated responses to the requested IDs.
/// The coordinator additionally checks chunk sizes before growing any buffer.
/// Unit tests exercise this contract without networking or credentials.
pub(crate) trait HostedTransport {
    fn create_once(
        &mut self,
        request: &[u8],
        deadline: Instant,
        event: &mut Receive<'_>,
    ) -> Result<(), CoreError>;
    fn read(
        &mut self,
        resource: HostedRead<'_>,
        deadline: Instant,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError>;
    fn delete_session(
        &mut self,
        session_id: &str,
        deadline: Instant,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError>;
}

struct PinnedRequest {
    // Kept alive through export, provider verification and output import.
    workspace: OwnedFd,
    task: OwnedFd,
    outputs: OwnedFd,
    request: Zeroizing<Vec<u8>>,
    snapshot_digest: String,
}

fn directory_identity(fd: &OwnedFd) -> Result<(u64, u64), CoreError> {
    let file = File::from(fd.try_clone()?);
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

fn pin_request(context: &HostedRunContext<'_>) -> Result<PinnedRequest, CoreError> {
    if context.storage_namespace.is_empty()
        || context.storage_namespace.len() > 4096
        || context.storage_namespace.chars().any(char::is_control)
        || context.storage_revision == 0
        || context.storage_revision == u64::MAX
    {
        return Err(fail("ERR_HOSTED_WORKSPACE_BINDING"));
    }
    let workspace = open_absolute_root(context.workspace_root)?;
    let task_path = agent_task_root(&context.manifest.task_id)?;
    let components: Vec<_> = task_path.split('/').collect();
    let task = require_existing_chain(&workspace, &components)?;
    let outputs = require_existing_chain(&task, &["outputs"])?;
    require_empty_pinned_output_directory(&outputs)?;
    let bundle = build_sandbox_task_bundle(context.manifest, context.reviewed_brief)?;
    if bundle.requested_outputs.len() > MAX_OUTPUT_FILES {
        return Err(fail("ERR_HOSTED_OUTPUT_SCOPE"));
    }
    // Prove that the requested names can coexist in the common importer
    // BEFORE a charged create, including portable/case/prefix conflicts.
    validate_sandbox_output_listing(&SandboxRemoteOutputListing {
        schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
        entries: bundle
            .requested_outputs
            .iter()
            .map(|path| SandboxRemoteOutputEntry {
                path: path.clone(),
                kind: SandboxRemoteEntryKind::File,
                byte_size: 0,
                sha256: format!("{:x}", Sha256::digest([])),
            })
            .collect(),
    })?;
    let request = build_verified_hosted_request_from_pinned_task(
        &task,
        context.manifest,
        context.reviewed_brief,
        context.model,
    )?;
    let request = Zeroizing::new(serde_json::to_vec(&request)?);
    if request.len() > OPENAI_HOSTED_REQUEST_MAX_JSON_BYTES {
        return Err(fail("ERR_HOSTED_REQUEST_BUDGET"));
    }
    // Consent binds actual bytes via the exact request hash, output scope,
    // namespace, pinned native directories and immutable provider disclosures.
    // Native paths and request contents are never persisted in the record.
    let binding = serde_json::to_vec(&(
        "infimount:v0.8.2:hosted-export-consent:v1",
        &context.manifest.workspace_id,
        &context.manifest.task_id,
        context.storage_namespace,
        context.storage_revision,
        directory_identity(&task)?,
        directory_identity(&outputs)?,
        bundle.bundle_metadata_sha256,
        format!("{:x}", Sha256::digest(request.as_slice())),
        (
            "openai_agents_api",
            "us_only",
            "non_zdr",
            100_u32,
            "network_disabled",
        ),
    ))?;
    Ok(PinnedRequest {
        workspace,
        task,
        outputs,
        request,
        snapshot_digest: format!("{:x}", Sha256::digest(binding)),
    })
}

impl PinnedRequest {
    /// Detect an already detached task before writing or claiming success.
    /// These reopened handles are observations only. All actual reads/writes
    /// continue through the original pinned handles, including after this
    /// check, so a later replacement cannot redirect effects. Same-UID rename
    /// prevention is not claimed by this private prototype.
    fn require_attached(&self, context: &HostedRunContext<'_>) -> Result<(), CoreError> {
        let current_workspace = open_absolute_root(context.workspace_root)?;
        let task_path = agent_task_root(&context.manifest.task_id)?;
        let components: Vec<_> = task_path.split('/').collect();
        let current_task = require_existing_chain(&self.workspace, &components)?;
        let current_outputs = require_existing_chain(&self.task, &["outputs"])?;
        if directory_identity(&current_workspace)? != directory_identity(&self.workspace)?
            || directory_identity(&current_task)? != directory_identity(&self.task)?
            || directory_identity(&current_outputs)? != directory_identity(&self.outputs)?
        {
            return Err(fail("ERR_HOSTED_TASK_DETACHED"));
        }
        Ok(())
    }
}

fn receive_bounded(
    max_bytes: usize,
    deadline: Instant,
    operation: impl FnOnce(&mut Receive<'_>) -> Result<(), CoreError>,
) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    check_deadline(deadline)?;
    let mut bytes = Zeroizing::new(Vec::new());
    let mut rejected = false;
    let result = operation(&mut |chunk| {
        if rejected
            || check_deadline(deadline).is_err()
            || chunk.len() > max_bytes.saturating_sub(bytes.len())
        {
            rejected = true;
            return Err(fail("ERR_HOSTED_RESPONSE_BUDGET"));
        }
        bytes.extend_from_slice(chunk);
        Ok(())
    });
    if rejected || result.is_err() {
        return Err(fail("ERR_HOSTED_RESPONSE"));
    }
    check_deadline(deadline)?;
    Ok(bytes)
}

fn read_bounded(
    transport: &mut impl HostedTransport,
    resource: HostedRead<'_>,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    receive_bounded(max_bytes, deadline, |receive| {
        transport.read(resource, deadline, receive)
    })
}

struct CreatedSession {
    session_id: Option<String>,
    environment_id: Option<String>,
    turn_id: Option<String>,
    events: HostedSessionEventGate,
    event_count: usize,
    bytes: usize,
    rejected: bool,
}

impl CreatedSession {
    fn new() -> Self {
        Self {
            session_id: None,
            environment_id: None,
            turn_id: None,
            events: HostedSessionEventGate::new(),
            event_count: 0,
            bytes: 0,
            rejected: false,
        }
    }

    fn observe(
        &mut self,
        bytes: &[u8],
        records: &ExecutionRecordStore,
        running: &mut AgentTaskExecutionRecord,
        deadline: Instant,
    ) -> Result<(), CoreError> {
        if self.rejected {
            return Err(fail("ERR_HOSTED_EVENTS"));
        }
        let result = self.observe_inner(bytes, records, running, deadline);
        if result.is_err() {
            self.rejected = true;
        }
        result
    }

    fn observe_inner(
        &mut self,
        bytes: &[u8],
        records: &ExecutionRecordStore,
        running: &mut AgentTaskExecutionRecord,
        deadline: Instant,
    ) -> Result<(), CoreError> {
        check_deadline(deadline)?;
        if self.event_count >= MAX_EVENTS
            || bytes.len() > MAX_JSON_BYTES
            || bytes.len() > MAX_STREAM_BYTES.saturating_sub(self.bytes)
        {
            return Err(fail("ERR_HOSTED_EVENTS"));
        }
        self.event_count += 1;
        self.bytes += bytes.len();
        let value: Value = serde_json::from_slice(bytes)?;
        match value["type"].as_str() {
            Some("agent.session.created") => {
                let id = value["session"]["id"]
                    .as_str()
                    .filter(|id| safe_id(id))
                    .ok_or_else(|| fail("ERR_HOSTED_SESSION_ID"))?;
                if self.session_id.is_some() {
                    return Err(fail("ERR_HOSTED_SESSION_ID"));
                }
                // Retain the ID even if durable binding fails so this process
                // can attempt cleanup. No further event is accepted on failure.
                self.session_id = Some(id.into());
                *running = bind_hosted_session_once(records, running, id)?;
                if !value["session"]["environment"]["id"].is_null() {
                    let id = value["session"]["environment"]["id"]
                        .as_str()
                        .filter(|id| safe_id(id))
                        .ok_or_else(|| fail("ERR_HOSTED_ENVIRONMENT"))?;
                    self.environment_id = Some(id.into());
                }
            }
            Some("agent.session.turn.completed")
                if value["turn"].get("subagent_id").is_some_and(Value::is_null) =>
            {
                if self.session_id.is_none() || self.turn_id.is_some() {
                    return Err(fail("ERR_HOSTED_ROOT_TURN"));
                }
                let id = value["turn"]["id"]
                    .as_str()
                    .filter(|id| safe_id(id))
                    .ok_or_else(|| fail("ERR_HOSTED_ROOT_TURN"))?;
                self.turn_id = Some(id.into());
            }
            Some("agent.session.requires_action" | "agent.session.action_required") => {
                return Err(fail("ERR_HOSTED_REQUIRED_ACTION"));
            }
            _ => {}
        }
        self.events.observe_json(bytes)?;
        Ok(())
    }
}

struct DownloadedOutput {
    artifact_id: String,
    task_path: String,
    bytes: Zeroizing<Vec<u8>>,
}

struct VerifiedOutputs {
    candidate: SandboxOutputCandidatePlan,
    files: Vec<DownloadedOutput>,
}

fn inspect_and_download(
    transport: &mut impl HostedTransport,
    context: &HostedRunContext<'_>,
    observed: &mut CreatedSession,
    deadline: Instant,
) -> Result<VerifiedOutputs, CoreError> {
    let outcome = observed.events.finish_stream();
    if observed.rejected || outcome != HostedSessionObservedOutcome::RootTurnCompletedUnverified {
        return Err(fail("ERR_HOSTED_UNVERIFIED_TURN"));
    }
    let session_id = observed
        .session_id
        .as_deref()
        .ok_or_else(|| fail("ERR_HOSTED_SESSION_ID"))?;
    let turn_id = observed
        .turn_id
        .as_deref()
        .ok_or_else(|| fail("ERR_HOSTED_ROOT_TURN"))?;
    let session_bytes = read_bounded(
        transport,
        HostedRead::Session { session_id },
        MAX_JSON_BYTES,
        deadline,
    )?;
    let session: Value = serde_json::from_slice(&session_bytes)?;
    let environment_id = session["environment"]["id"]
        .as_str()
        .filter(|id| safe_id(id))
        .ok_or_else(|| fail("ERR_HOSTED_ENVIRONMENT"))?;
    if session["id"].as_str() != Some(session_id)
        || session["status"].as_str() != Some("idle")
        || observed
            .environment_id
            .as_deref()
            .is_some_and(|id| id != environment_id)
    {
        return Err(fail("ERR_HOSTED_SESSION_BINDING"));
    }

    let mut audit = HostedSavedItemAudit::new(turn_id)?;
    for _ in 0..MAX_HOSTED_ITEM_PAGES {
        let after = audit.next_after().map(str::to_owned);
        let page = read_bounded(
            transport,
            HostedRead::SavedItems {
                session_id,
                turn_id,
                after: after.as_deref(),
            },
            MAX_JSON_BYTES,
            deadline,
        )?;
        audit.observe_page(&page, after.as_deref())?;
        if audit.next_after().is_none() {
            break;
        }
    }
    audit.finish(outcome)?;

    let mut inventory = HostedArtifactInventory::new(
        context.manifest,
        context.reviewed_brief,
        session_id,
        turn_id,
        environment_id,
    )?;
    for _ in 0..MAX_HOSTED_ARTIFACT_PAGES {
        let after = inventory.next_after().map(str::to_owned);
        let page = read_bounded(
            transport,
            HostedRead::Artifacts {
                session_id,
                after: after.as_deref(),
            },
            MAX_JSON_BYTES,
            deadline,
        )?;
        inventory.observe_page(&page, after.as_deref())?;
        if inventory.next_after().is_none() {
            break;
        }
    }
    let plan = inventory.finish(outcome)?;
    if plan.files().len() > MAX_OUTPUT_FILES || plan.total_bytes() > MAX_OUTPUT_BYTES as u64 {
        return Err(fail("ERR_HOSTED_OUTPUT_BUDGET"));
    }
    let mut files = Vec::with_capacity(plan.files().len());
    for file in plan.files() {
        let bytes = read_bounded(
            transport,
            HostedRead::ArtifactBytes {
                session_id,
                artifact_id: &file.id,
            },
            file.size_bytes as usize,
            deadline,
        )?;
        files.push(DownloadedOutput {
            artifact_id: file.id.clone(),
            task_path: file.task_path.clone(),
            bytes,
        });
    }
    let borrowed: Vec<_> = files
        .iter()
        .map(|file| DownloadedHostedArtifact {
            artifact_id: &file.artifact_id,
            bytes: &file.bytes,
        })
        .collect();
    let candidate = validate_downloaded_hosted_artifacts(&plan, &borrowed)?;
    Ok(VerifiedOutputs { candidate, files })
}

fn confirm_cleanup(
    transport: &mut impl HostedTransport,
    session_id: &str,
) -> Result<(), CoreError> {
    let deadline = Instant::now() + CLEANUP_TIMEOUT;
    let bytes = receive_bounded(MAX_JSON_BYTES, deadline, |receive| {
        transport.delete_session(session_id, deadline, receive)
    })?;
    let value: Value = serde_json::from_slice(&bytes)?;
    if value["id"].as_str() != Some(session_id)
        || value["object"].as_str() != Some("agent.session.deleted")
        || value["deleted"].as_bool() != Some(true)
    {
        return Err(fail("ERR_HOSTED_CLEANUP_UNCONFIRMED"));
    }
    Ok(())
}

fn record_failed(
    context: &HostedRunContext<'_>,
    running: &AgentTaskExecutionRecord,
    clock: &impl HostedClock,
    code: &'static str,
) -> Result<(), CoreError> {
    let mut failed = running.clone();
    failed.status = ExecutionStatus::Failed;
    failed.finished_at = Some(clock.rfc3339());
    failed.error_code = Some(code.into());
    context.records.transition(running, &failed)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HostedRunSummary {
    pub execution_id: String,
    pub output_file_count: usize,
    pub output_total_bytes: u64,
}

/// Consume a separately authorized exact snapshot and run the private
/// lifecycle. Caller locks must span this whole operation. No callback or
/// response can authorize original-storage publication.
pub(crate) fn run_hosted_execution(
    context: &HostedRunContext<'_>,
    consent: Option<HostedExportConsent>,
    transport: &mut impl HostedTransport,
    clock: &impl HostedClock,
) -> Result<HostedRunSummary, CoreError> {
    let consent = consent.ok_or_else(|| fail("ERR_HOSTED_CONSENT"))?;
    let pinned = pin_request(context).map_err(|_| fail("ERR_HOSTED_INPUT_SCOPE"))?;
    let consent_issued_at = consent.issued_at_ms;
    let consent_expires_at = consent.expires_at_ms;
    consent.consume(&pinned.snapshot_digest, clock.unix_ms())?;
    let prepared = prepare_hosted_execution(
        context.records,
        context.manifest,
        context.reviewed_brief,
        context.model,
        &clock.rfc3339(),
    )
    .map_err(|_| fail("ERR_HOSTED_PREPARE"))?;
    let mut running = mark_hosted_create_started(context.records, &prepared)
        .map_err(|_| fail("ERR_HOSTED_START_EVIDENCE"))?;
    let submit_time = clock.unix_ms();
    if submit_time < consent_issued_at || submit_time >= consent_expires_at {
        // The create has not been attempted, so closing this local attempt is
        // safe. Recheck AFTER durable-store operations, immediately at submit.
        record_failed(context, &running, clock, "ERR_HOSTED_CONSENT_EXPIRED")
            .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
        return Err(fail("ERR_HOSTED_CONSENT_EXPIRED"));
    }
    let deadline = Instant::now() + RUN_TIMEOUT;
    let mut observed = CreatedSession::new();
    let created = transport.create_once(&pinned.request, deadline, &mut |bytes| {
        observed.observe(bytes, context.records, &mut running, deadline)
    });
    let result =
        created.and_then(|()| inspect_and_download(transport, context, &mut observed, deadline));
    // Run cleanup even when parsing, persistence, download or stream handling
    // failed. An ambiguous create or failed delete must stay Running so the
    // existing private store blocks an automatic second paid create.
    let Some(session_id) = observed.session_id.as_deref() else {
        return Err(fail("ERR_HOSTED_MANUAL_RECONCILIATION"));
    };
    if confirm_cleanup(transport, session_id).is_err() {
        return Err(fail("ERR_HOSTED_CLEANUP_REQUIRED"));
    }
    // The remote API confirms public deletion, not immediate physical erasure.
    // Persist separate create-only evidence BEFORE import or terminal status.
    // Without durable evidence, retain Running and block a second create.
    persist_public_deletion(context.records, &running, &clock.rfc3339())
        .map_err(|_| fail("ERR_HOSTED_DELETION_RECEIPT"))?;
    let verified = match result {
        Ok(verified) => verified,
        Err(_) => {
            record_failed(context, &running, clock, "ERR_HOSTED_VERIFICATION")
                .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
            return Err(fail("ERR_HOSTED_VERIFICATION"));
        }
    };
    let staged: Vec<_> = verified
        .files
        .iter()
        .map(|file| VerifiedStagedOutput {
            task_path: &file.task_path,
            bytes: &file.bytes,
        })
        .collect();
    if pinned.require_attached(context).is_err() {
        record_failed(context, &running, clock, "ERR_HOSTED_TASK_DETACHED")
            .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
        return Err(fail("ERR_HOSTED_TASK_DETACHED"));
    }
    let summary = match ingest_reviewed_outputs_to_pinned_directory(
        &pinned.outputs,
        context.manifest,
        context.reviewed_brief,
        &verified.candidate,
        &staged,
    ) {
        Ok(summary) => summary,
        Err(error) => {
            let code = if matches!(error, CoreError::TransferCleanupRequired) {
                "ERR_HOSTED_IMPORT_CLEANUP_REQUIRED"
            } else {
                "ERR_HOSTED_IMPORT"
            };
            record_failed(context, &running, clock, code)
                .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
            return Err(fail(code));
        }
    };
    let mut succeeded = running.clone();
    if pinned.require_attached(context).is_err() {
        // Verified files may exist in the original pinned tree. Never remove
        // them or report success after observing that it became detached.
        record_failed(
            context,
            &running,
            clock,
            "ERR_HOSTED_IMPORT_CLEANUP_REQUIRED",
        )
        .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
        return Err(fail("ERR_HOSTED_IMPORT_CLEANUP_REQUIRED"));
    }
    succeeded.status = ExecutionStatus::Succeeded;
    succeeded.finished_at = Some(clock.rfc3339());
    succeeded.output_file_count = summary.file_count as u32;
    succeeded.output_total_bytes = summary.total_bytes;
    // The candidate digest binds canonical paths, lengths and measured hashes.
    // The importer independently rehashed those same bytes after create.
    succeeded.output_aggregate_sha256 = Some(verified.candidate.listing_metadata_sha256);
    context
        .records
        .transition(&running, &succeeded)
        .map_err(|_| fail("ERR_HOSTED_COMPLETION_EVIDENCE"))?;
    Ok(HostedRunSummary {
        execution_id: running.execution_id,
        output_file_count: summary.file_count,
        output_total_bytes: summary.total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted_cleanup_evidence::public_deletion_confirmed;
    use crate::hosted_execution_recovery::{recover_hosted_attempt, HostedRecoveryAction};
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use tempfile::TempDir;

    const WORKSPACE: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";
    const TASK: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
    const SESSION: &str = "sess_synthetic";
    const TURN: &str = "turn_synthetic";
    const ENVIRONMENT: &str = "env_synthetic";
    const OUTPUT: &[u8] = b"report";

    struct Clock;
    impl HostedClock for Clock {
        fn unix_ms(&self) -> u64 {
            10_000
        }
        fn rfc3339(&self) -> String {
            "2026-11-17T07:00:00Z".into()
        }
    }

    struct Fixture {
        _private: TempDir,
        workspace: TempDir,
        records: ExecutionRecordStore,
        manifest: AgentTaskManifest,
        brief: AgentTaskBrief,
    }

    impl Fixture {
        fn new() -> Self {
            let private = TempDir::new().unwrap();
            let workspace = TempDir::new().unwrap();
            let task = workspace.path().join(format!("tasks/{TASK}"));
            fs::create_dir_all(task.join("inputs")).unwrap();
            fs::create_dir_all(task.join("outputs")).unwrap();
            fs::write(task.join("inputs/data.txt"), b"alpha").unwrap();
            let manifest = AgentTaskManifest {
                schema_version: AGENT_TASK_SCHEMA_VERSION,
                task_id: TASK.into(),
                workspace_id: WORKSPACE.into(),
                task_root: format!("tasks/{TASK}"),
                outputs_directory: "outputs".into(),
                title: "PRIVATE original source title".into(),
                created_at: "2026-11-17T06:00:00Z".into(),
                inputs: vec![AgentTaskInput {
                    task_path: "inputs/data.txt".into(),
                    byte_size: 5,
                    sha256: format!("{:x}", Sha256::digest(b"alpha")),
                }],
            };
            fs::write(
                task.join("task-manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            Self {
                records: ExecutionRecordStore::new(private.path()),
                _private: private,
                workspace,
                manifest,
                brief: AgentTaskBrief {
                    title: "Reviewed synthetic task".into(),
                    objective: "Write the approved report.".into(),
                    requested_outputs: vec!["outputs/report.txt".into()],
                },
            }
        }

        fn context(&self) -> HostedRunContext<'_> {
            HostedRunContext {
                workspace_root: self.workspace.path(),
                manifest: &self.manifest,
                reviewed_brief: &self.brief,
                model: "gpt-6-astra",
                storage_namespace: "trusted-local-namespace",
                storage_revision: 1,
                records: &self.records,
            }
        }

        fn consent(&self) -> HostedExportConsent {
            HostedExportConsent::fixture(
                pin_request(&self.context()).unwrap().snapshot_digest,
                Clock.unix_ms(),
            )
        }

        fn task(&self) -> PathBuf {
            self.workspace.path().join(format!("tasks/{TASK}"))
        }
        fn output(&self) -> PathBuf {
            self.task().join("outputs/report.txt")
        }
        fn record(&self) -> AgentTaskExecutionRecord {
            let records = self.records.list_for_task(WORKSPACE, TASK).unwrap();
            assert_eq!(records.len(), 1);
            records[0].clone()
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        UnknownCreate,
        EvidenceConflict,
        DisconnectAfterId,
        WrongSession,
        WrongTurn,
        ToolExit,
        CursorLoop,
        ExtraArtifact,
        OversizedDownload,
        CleanupFailure,
        CompletionConflict,
        LateCollision,
        LateSymlink,
        PartialImport,
        LateStreamFailure,
        ReplaceTask,
        ReplaceOutputs,
    }

    struct MockProvider {
        fault: Fault,
        records: ExecutionRecordStore,
        task: PathBuf,
        calls: Vec<&'static str>,
    }

    impl MockProvider {
        fn new(fixture: &Fixture, fault: Fault) -> Self {
            Self {
                fault,
                records: fixture.records.clone(),
                task: fixture.task(),
                calls: vec![],
            }
        }
        fn emit(value: Value, receive: &mut Receive<'_>) -> Result<(), CoreError> {
            receive(&serde_json::to_vec(&value).unwrap())
        }
    }

    impl HostedTransport for MockProvider {
        fn create_once(
            &mut self,
            request: &[u8],
            deadline: Instant,
            event: &mut Receive<'_>,
        ) -> Result<(), CoreError> {
            check_deadline(deadline)?;
            self.calls.push("create");
            let records = self.records.list_for_task(WORKSPACE, TASK).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].status, ExecutionStatus::Running);
            assert!(records[0].provider_execution_id.is_none());
            let wire: Value = serde_json::from_slice(request).unwrap();
            assert_eq!(wire["environment"]["network"]["access"], "disabled");
            assert_eq!(wire["spend_control"]["limit"], 100);
            assert_eq!(wire["environment"]["files"][0]["data"], "YWxwaGE=");
            assert!(!String::from_utf8_lossy(request).contains("PRIVATE"));
            if self.fault == Fault::UnknownCreate {
                return Err(fail("PROVIDER_BODY_WITH_PRIVATE_ERROR"));
            }
            if self.fault == Fault::EvidenceConflict {
                // Simulate a conflicting durable update at the first-ID CAS.
                let mut changed = records[0].clone();
                changed.status = ExecutionStatus::Failed;
                changed.finished_at = Some(Clock.rfc3339());
                changed.error_code = Some("ERR_TEST_CONCURRENT_UPDATE".into());
                self.records.transition(&records[0], &changed).unwrap();
            }
            Self::emit(
                json!({"type":"agent.session.created", "session":{
                    "id":SESSION, "environment":{"id":ENVIRONMENT}
                }}),
                event,
            )?;
            // This observation happens before the mock supplies another event.
            let bound = self.records.list_for_task(WORKSPACE, TASK).unwrap();
            assert_eq!(bound[0].provider_execution_id.as_deref(), Some(SESSION));
            if self.fault == Fault::DisconnectAfterId {
                return Err(fail("PROVIDER_BODY_WITH_PRIVATE_ERROR"));
            }
            if self.fault == Fault::ReplaceTask {
                fs::rename(&self.task, self.task.with_file_name("moved-task")).unwrap();
                fs::create_dir_all(self.task.join("outputs")).unwrap();
            }
            if self.fault == Fault::ReplaceOutputs {
                fs::rename(self.task.join("outputs"), self.task.join("moved-outputs")).unwrap();
                fs::create_dir(self.task.join("outputs")).unwrap();
            }
            Self::emit(
                json!({"type":"agent.session.turn.completed", "turn":{
                    "id":TURN, "subagent_id":null
                }}),
                event,
            )?;
            if self.fault == Fault::LateStreamFailure {
                Self::emit(json!({"type":"agent.session.failed"}), event)?;
            }
            Ok(())
        }

        fn read(
            &mut self,
            resource: HostedRead<'_>,
            deadline: Instant,
            receive: &mut Receive<'_>,
        ) -> Result<(), CoreError> {
            check_deadline(deadline)?;
            match resource {
                HostedRead::Session { session_id } => {
                    self.calls.push("session");
                    assert_eq!(session_id, SESSION);
                    Self::emit(
                        json!({"id":if self.fault == Fault::WrongSession {"sess_foreign"} else {SESSION},
                        "status":"idle", "environment":{"id":ENVIRONMENT}}),
                        receive,
                    )
                }
                HostedRead::SavedItems {
                    session_id,
                    turn_id,
                    after,
                } => {
                    self.calls.push("items");
                    assert_eq!((session_id, turn_id), (SESSION, TURN));
                    assert!(after.is_none() || self.fault == Fault::CursorLoop);
                    Self::emit(
                        json!({"object":"list", "has_more":self.fault == Fault::CursorLoop,
                        "first_id":"item_command", "last_id":"opaque_cursor",
                        "data":[{"id":"item_command", "turn_id":if self.fault == Fault::WrongTurn {"turn_foreign"} else {TURN},
                            "type":"command_execution", "status":"completed",
                            "exit_code":if self.fault == Fault::ToolExit {1} else {0}}]}),
                        receive,
                    )
                }
                HostedRead::Artifacts { session_id, after } => {
                    self.calls.push("artifacts");
                    assert_eq!(session_id, SESSION);
                    assert!(after.is_none());
                    if self.fault == Fault::PartialImport {
                        return Self::emit(
                            json!({"object":"list", "has_more":false,
                            "first_id":"artifact_a", "last_id":"artifact_z",
                            "data":[
                                {"id":"artifact_a", "object":"agent.session.artifact", "session_id":SESSION,
                                 "turn_id":TURN, "environment_id":ENVIRONMENT, "path":"/workspace/outputs/a.txt", "size_bytes":OUTPUT.len()},
                                {"id":"artifact_z", "object":"agent.session.artifact", "session_id":SESSION,
                                 "turn_id":TURN, "environment_id":ENVIRONMENT, "path":"/workspace/outputs/z/report.txt", "size_bytes":OUTPUT.len()}
                            ]}),
                            receive,
                        );
                    }
                    Self::emit(
                        json!({"object":"list", "has_more":false, "first_id":"artifact_report", "last_id":"artifact_report",
                        "data":[{"id":"artifact_report", "object":"agent.session.artifact",
                            "session_id":SESSION, "turn_id":TURN, "environment_id":ENVIRONMENT,
                            "path":if self.fault == Fault::ExtraArtifact {"/workspace/outputs/extra.txt"} else {"/workspace/outputs/report.txt"},
                            "size_bytes":OUTPUT.len()}]}),
                        receive,
                    )
                }
                HostedRead::ArtifactBytes {
                    session_id,
                    artifact_id,
                } => {
                    self.calls.push("download");
                    assert_eq!(session_id, SESSION);
                    if self.fault == Fault::PartialImport {
                        assert!(["artifact_a", "artifact_z"].contains(&artifact_id));
                    } else {
                        assert_eq!(artifact_id, "artifact_report");
                    }
                    receive(OUTPUT)?;
                    if self.fault == Fault::OversizedDownload {
                        receive(b"x")?;
                    }
                    Ok(())
                }
            }
        }

        fn delete_session(
            &mut self,
            session_id: &str,
            deadline: Instant,
            receive: &mut Receive<'_>,
        ) -> Result<(), CoreError> {
            check_deadline(deadline)?;
            self.calls.push("delete");
            assert_eq!(session_id, SESSION);
            // Cleanup must finish before any new output is imported.
            assert!(
                !self.task.join("outputs/report.txt").exists()
                    || fs::read(self.task.join("outputs/report.txt")).unwrap() == b"sentinel"
            );
            if self.fault == Fault::CleanupFailure {
                return Err(fail("PROVIDER_BODY_WITH_PRIVATE_ERROR"));
            }
            if self.fault == Fault::LateCollision {
                fs::write(self.task.join("outputs/report.txt"), b"sentinel").unwrap();
            }
            if self.fault == Fault::LateSymlink {
                let sentinel = self.task.join("sentinel.txt");
                fs::write(&sentinel, b"sentinel").unwrap();
                symlink(sentinel, self.task.join("outputs/report.txt")).unwrap();
            }
            if self.fault == Fault::PartialImport {
                use std::os::unix::fs::PermissionsExt;
                let directory = self.task.join("outputs/z");
                fs::create_dir(&directory).unwrap();
                fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
            }
            if self.fault == Fault::CompletionConflict {
                let records = self.records.list_for_task(WORKSPACE, TASK).unwrap();
                let mut changed = records[0].clone();
                changed.status = ExecutionStatus::Failed;
                changed.finished_at = Some(Clock.rfc3339());
                changed.error_code = Some("ERR_TEST_CONCURRENT_UPDATE".into());
                self.records.transition(&records[0], &changed).unwrap();
            }
            Self::emit(
                json!({"id":SESSION, "object":"agent.session.deleted", "deleted":true}),
                receive,
            )
        }
    }

    #[test]
    fn successful_delete_requires_durable_public_receipt() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::None);
        run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .unwrap();
        assert!(public_deletion_confirmed(&fixture.records, &fixture.record()).unwrap());
    }

    #[test]
    fn failed_delete_never_writes_public_receipt() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::CleanupFailure);
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock
        )
        .is_err());
        assert!(!public_deletion_confirmed(&fixture.records, &fixture.record()).unwrap());
    }

    #[test]
    fn real_files_flow_through_one_create_durable_binding_cleanup_import_and_completion() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::None);
        let result = run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .unwrap();
        assert_eq!(
            provider.calls,
            [
                "create",
                "session",
                "items",
                "artifacts",
                "download",
                "delete"
            ]
        );
        assert_eq!(fs::read(fixture.output()).unwrap(), OUTPUT);
        assert_eq!(
            fs::read(fixture.task().join("inputs/data.txt")).unwrap(),
            b"alpha"
        );
        assert_eq!(result.output_file_count, 1);
        assert_eq!(result.output_total_bytes, OUTPUT.len() as u64);
        let record = fixture.record();
        assert_eq!(record.status, ExecutionStatus::Succeeded);
        assert_eq!(record.output_file_count, 1);
        assert_eq!(record.execution_id, result.execution_id);
        assert!(record.output_aggregate_sha256.is_some());
        let evidence = serde_json::to_string(&record).unwrap();
        for forbidden in [
            "PRIVATE",
            "alpha",
            "report.txt",
            "Write the approved",
            "trusted-local-namespace",
        ] {
            assert!(!evidence.contains(forbidden));
        }
    }

    #[test]
    fn absent_expired_or_scope_changed_consent_cannot_create_or_write() {
        for variant in 0..7 {
            let mut fixture = Fixture::new();
            let mut consent = fixture.consent();
            match variant {
                1 => consent.expires_at_ms = Clock.unix_ms(),
                2 => fixture.brief.objective = "Changed after review".into(),
                3 => fs::write(fixture.task().join("inputs/data.txt"), b"omega").unwrap(),
                4 => consent.snapshot_digest = "0".repeat(64),
                5 => fixture.brief.title = "Changed title after review".into(),
                6 => fixture.brief.requested_outputs = vec!["outputs/changed.txt".into()],
                _ => {}
            }
            let mut provider = MockProvider::new(&fixture, Fault::None);
            assert!(run_hosted_execution(
                &fixture.context(),
                (variant != 0).then_some(consent),
                &mut provider,
                &Clock
            )
            .is_err());
            assert!(provider.calls.is_empty());
            assert!(!fixture.output().exists());
            assert!(fixture
                .records
                .list_for_task(WORKSPACE, TASK)
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn ambiguous_create_stays_running_and_blocks_a_second_create() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::UnknownCreate);
        let error = run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("PROVIDER_BODY"));
        let record = fixture.record();
        assert_eq!(record.status, ExecutionStatus::Running);
        assert!(record.provider_execution_id.is_none());
        assert_eq!(
            recover_hosted_attempt(&fixture.records, WORKSPACE, TASK, &record.execution_id)
                .unwrap(),
            HostedRecoveryAction::ManualReconciliationRequired
        );
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock
        )
        .is_err());
        assert_eq!(provider.calls, ["create"]);
        assert!(!fixture.output().exists());
    }

    #[test]
    fn expired_while_persisting_evidence_never_submits_to_provider() {
        struct AdvancingClock(std::cell::Cell<usize>);
        impl HostedClock for AdvancingClock {
            fn unix_ms(&self) -> u64 {
                let calls = self.0.get();
                self.0.set(calls + 1);
                Clock.unix_ms()
                    + if calls == 0 {
                        0
                    } else {
                        MAX_CONSENT_LIFETIME_MS
                    }
            }
            fn rfc3339(&self) -> String {
                Clock.rfc3339()
            }
        }
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::None);
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &AdvancingClock(std::cell::Cell::new(0)),
        )
        .is_err());
        assert!(provider.calls.is_empty());
        let record = fixture.record();
        assert_eq!(record.status, ExecutionStatus::Failed);
        assert_eq!(
            record.error_code.as_deref(),
            Some("ERR_HOSTED_CONSENT_EXPIRED")
        );
        assert!(!fixture.output().exists());
    }

    #[test]
    fn failed_durable_id_binding_does_not_suppress_observed_session_cleanup() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::EvidenceConflict);
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .is_err());
        assert_eq!(provider.calls, ["create", "delete"]);
        assert!(!fixture.output().exists());
    }

    #[test]
    fn failed_completion_cas_preserves_imported_bytes_without_claiming_success() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::CompletionConflict);
        let error = run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .unwrap_err();
        assert!(error.to_string().contains("ERR_HOSTED_COMPLETION_EVIDENCE"));
        assert_eq!(fs::read(fixture.output()).unwrap(), OUTPUT);
        assert_eq!(fixture.record().status, ExecutionStatus::Failed);
        assert_eq!(provider.calls.last(), Some(&"delete"));
    }

    #[test]
    fn provider_verification_failures_cleanup_the_bound_session_without_import() {
        for fault in [
            Fault::DisconnectAfterId,
            Fault::WrongSession,
            Fault::WrongTurn,
            Fault::ToolExit,
            Fault::CursorLoop,
            Fault::ExtraArtifact,
            Fault::OversizedDownload,
            Fault::LateStreamFailure,
        ] {
            let fixture = Fixture::new();
            let mut provider = MockProvider::new(&fixture, fault);
            let error = run_hosted_execution(
                &fixture.context(),
                Some(fixture.consent()),
                &mut provider,
                &Clock,
            )
            .unwrap_err();
            assert!(!error.to_string().contains("PROVIDER_BODY"));
            assert_eq!(
                provider
                    .calls
                    .iter()
                    .filter(|call| **call == "create")
                    .count(),
                1
            );
            assert_eq!(provider.calls.last(), Some(&"delete"));
            assert!(!fixture.output().exists());
            let record = fixture.record();
            assert_eq!(record.status, ExecutionStatus::Failed);
            assert_eq!(record.provider_execution_id.as_deref(), Some(SESSION));
            assert_eq!(
                record.error_code.as_deref(),
                Some("ERR_HOSTED_VERIFICATION")
            );
        }
    }

    #[test]
    fn cleanup_failure_keeps_pinned_recovery_and_no_import_or_second_create() {
        let fixture = Fixture::new();
        let mut provider = MockProvider::new(&fixture, Fault::CleanupFailure);
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock
        )
        .is_err());
        assert!(!fixture.output().exists());
        let record = fixture.record();
        assert_eq!(record.status, ExecutionStatus::Running);
        assert_eq!(
            recover_hosted_attempt(&fixture.records, WORKSPACE, TASK, &record.execution_id)
                .unwrap(),
            HostedRecoveryAction::RecoverPinnedSession(SESSION.into())
        );
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock
        )
        .is_err());
        assert_eq!(
            provider
                .calls
                .iter()
                .filter(|call| **call == "create")
                .count(),
            1
        );
    }

    #[test]
    fn existing_or_linked_output_is_never_overwritten() {
        for kind in 0..3 {
            let fixture = Fixture::new();
            let consent = fixture.consent();
            let sentinel = fixture.workspace.path().join("sentinel.txt");
            fs::write(&sentinel, b"sentinel").unwrap();
            if kind == 1 {
                symlink(&sentinel, fixture.output()).unwrap();
            } else if kind == 2 {
                fs::create_dir(fixture.output()).unwrap();
            } else {
                fs::write(fixture.output(), b"sentinel").unwrap();
            }
            let mut provider = MockProvider::new(&fixture, Fault::None);
            assert!(
                run_hosted_execution(&fixture.context(), Some(consent), &mut provider, &Clock)
                    .is_err()
            );
            assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
            if kind != 2 {
                assert_eq!(fs::read(fixture.output()).unwrap(), b"sentinel");
            }
            assert!(fixture
                .records
                .list_for_task(WORKSPACE, TASK)
                .unwrap()
                .is_empty());
            assert!(provider.calls.is_empty());
        }
    }

    #[test]
    fn late_collision_or_symlink_is_rejected_after_cleanup_without_overwrite() {
        for fault in [Fault::LateCollision, Fault::LateSymlink] {
            let fixture = Fixture::new();
            let mut provider = MockProvider::new(&fixture, fault);
            assert!(run_hosted_execution(
                &fixture.context(),
                Some(fixture.consent()),
                &mut provider,
                &Clock
            )
            .is_err());
            assert_eq!(fs::read(fixture.output()).unwrap(), b"sentinel");
            assert_eq!(fixture.record().status, ExecutionStatus::Failed);
            assert_eq!(provider.calls.last(), Some(&"delete"));
        }
    }

    #[test]
    fn successful_task_cannot_charge_another_create_with_another_pending_consent() {
        let fixture = Fixture::new();
        let second_consent = fixture.consent();
        let mut provider = MockProvider::new(&fixture, Fault::None);
        run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        )
        .unwrap();
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(second_consent),
            &mut provider,
            &Clock
        )
        .is_err());
        assert_eq!(
            provider
                .calls
                .iter()
                .filter(|call| **call == "create")
                .count(),
            1
        );
        assert_eq!(fs::read(fixture.output()).unwrap(), OUTPUT);
        assert_eq!(fixture.record().status, ExecutionStatus::Succeeded);
    }

    #[test]
    fn impossible_reviewed_output_scope_is_rejected_before_create() {
        let mut fixture = Fixture::new();
        let consent = fixture.consent();
        fixture.brief.requested_outputs = vec!["outputs/a".into(), "outputs/a/b".into()];
        assert!(pin_request(&fixture.context()).is_err());
        let mut provider = MockProvider::new(&fixture, Fault::None);
        assert!(
            run_hosted_execution(&fixture.context(), Some(consent), &mut provider, &Clock).is_err()
        );
        assert!(provider.calls.is_empty());
    }

    #[test]
    fn partial_import_retains_committed_file_and_prevents_another_paid_attempt() {
        use std::os::unix::fs::PermissionsExt;
        // Root bypasses permission bits, so this EACCES fixture requires the
        // unprivileged user used by CI and ordinary desktop execution.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let mut fixture = Fixture::new();
        fixture.brief.requested_outputs =
            vec!["outputs/a.txt".into(), "outputs/z/report.txt".into()];
        let second_consent = fixture.consent();
        let mut provider = MockProvider::new(&fixture, Fault::PartialImport);
        let result = run_hosted_execution(
            &fixture.context(),
            Some(fixture.consent()),
            &mut provider,
            &Clock,
        );
        fs::set_permissions(
            fixture.task().join("outputs/z"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(result.is_err());
        assert_eq!(
            fs::read(fixture.task().join("outputs/a.txt")).unwrap(),
            OUTPUT
        );
        assert!(!fixture.task().join("outputs/z/report.txt").exists());
        assert_eq!(
            fixture.record().error_code.as_deref(),
            Some("ERR_HOSTED_IMPORT_CLEANUP_REQUIRED")
        );
        assert!(run_hosted_execution(
            &fixture.context(),
            Some(second_consent),
            &mut provider,
            &Clock
        )
        .is_err());
        assert_eq!(
            provider
                .calls
                .iter()
                .filter(|call| **call == "create")
                .count(),
            1
        );
    }

    #[test]
    fn replacement_task_or_outputs_cannot_redirect_pinned_import() {
        for fault in [Fault::ReplaceTask, Fault::ReplaceOutputs] {
            let fixture = Fixture::new();
            let mut provider = MockProvider::new(&fixture, fault);
            assert!(run_hosted_execution(
                &fixture.context(),
                Some(fixture.consent()),
                &mut provider,
                &Clock
            )
            .is_err());
            assert!(!fixture.output().exists());
            let pinned_path = if fault == Fault::ReplaceTask {
                fixture
                    .task()
                    .with_file_name("moved-task")
                    .join("outputs/report.txt")
            } else {
                fixture.task().join("moved-outputs/report.txt")
            };
            assert!(!pinned_path.exists());
            assert_eq!(
                fixture.record().error_code.as_deref(),
                Some("ERR_HOSTED_TASK_DETACHED")
            );
        }
    }

    #[test]
    fn namespace_revision_or_native_task_replacement_invalidates_pending_consent() {
        for variant in 0..2 {
            let fixture = Fixture::new();
            let consent = fixture.consent();
            let mut context = fixture.context();
            if variant == 0 {
                context.storage_revision += 1;
            } else {
                let old = fixture.task().with_file_name("old-task");
                fs::rename(fixture.task(), &old).unwrap();
                fs::create_dir_all(fixture.task().join("inputs")).unwrap();
                fs::create_dir_all(fixture.task().join("outputs")).unwrap();
                fs::copy(
                    old.join("task-manifest.json"),
                    fixture.task().join("task-manifest.json"),
                )
                .unwrap();
                fs::copy(
                    old.join("inputs/data.txt"),
                    fixture.task().join("inputs/data.txt"),
                )
                .unwrap();
            }
            let mut provider = MockProvider::new(&fixture, Fault::None);
            assert!(run_hosted_execution(&context, Some(consent), &mut provider, &Clock).is_err());
            assert!(provider.calls.is_empty());
        }
    }

    #[test]
    fn response_budget_stays_poisoned_even_if_transport_ignores_callback_error() {
        let result = receive_bounded(3, Instant::now() + Duration::from_secs(1), |receive| {
            assert!(receive(b"long").is_err());
            assert!(receive(b"ok").is_err());
            Ok(())
        });
        assert!(result.is_err());
    }
}
