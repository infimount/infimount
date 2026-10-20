//! Private, agent-inaccessible evidence for executions of an immutable Agent Task.
//! This is operational evidence (not a cryptographic attestation against the
//! local account owner), and is not exposed as an MCP resource or desktop tool.
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use chrono::DateTime;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_file::{self, FILE_MODE};
use crate::models::{CoreError, Result};

pub const EXECUTION_SCHEMA_VERSION: u32 = 1;
pub const PRIVATE_EXECUTIONS_DIR: &str = "agent-task-executions";
const MUTATION_LOCK: &str = ".mutations.lock";
const MAX_RECORD_BYTES: u64 = 32 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_EXECUTION_FILES: u32 = 1_000;
const MAX_EXECUTION_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const TASK_TOOLS: [TaskTool; 6] = [
    TaskTool::ListDir,
    TaskTool::StatPath,
    TaskTool::ReadFile,
    TaskTool::SearchPaths,
    TaskTool::WriteFile,
    TaskTool::Mkdir,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Codex,
    ClaudeCode,
    Opencode,
    GeminiCli,
    Pi,
    OpenaiSandbox,
    Daytona,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    LocalClient,
    RemoteSandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProvider {
    Openai,
    Daytona,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationProfile {
    McpOnlyReadonlyHost,
    RemoteBundleIsolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTool {
    ListDir,
    StatPath,
    ReadFile,
    SearchPaths,
    WriteFile,
    Mkdir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Prepared,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Version-1 evidence contract: all fields serialize, including explicit nulls.
/// Unknown fields are rejected and reading additionally requires all 21 keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskExecutionRecord {
    pub schema_version: u32,
    pub execution_id: String,
    pub task_id: String,
    pub workspace_id: String,
    pub adapter_kind: AdapterKind,
    pub adapter_version: String,
    pub executable_sha256: Option<String>,
    pub runtime_kind: RuntimeKind,
    pub runtime_provider: Option<RuntimeProvider>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: ExecutionStatus,
    pub task_scope_digest: String,
    pub enabled_task_tools: Vec<TaskTool>,
    pub isolation_profile: IsolationProfile,
    pub prompt_sha256: String,
    pub output_file_count: u32,
    pub output_total_bytes: u64,
    pub output_aggregate_sha256: Option<String>,
    pub error_code: Option<String>,
    pub provider_execution_id: Option<String>,
}

const REQUIRED_FIELDS: [&str; 21] = [
    "schemaVersion",
    "executionId",
    "taskId",
    "workspaceId",
    "adapterKind",
    "adapterVersion",
    "executableSha256",
    "runtimeKind",
    "runtimeProvider",
    "startedAt",
    "finishedAt",
    "status",
    "taskScopeDigest",
    "enabledTaskTools",
    "isolationProfile",
    "promptSha256",
    "outputFileCount",
    "outputTotalBytes",
    "outputAggregateSha256",
    "errorCode",
    "providerExecutionId",
];

fn invalid<T>(reason: &str) -> Result<T> {
    Err(CoreError::Config(format!("invalid Agent Task execution evidence: {reason}")))
}

fn valid_id(id: &str) -> bool {
    Uuid::parse_str(id)
        .is_ok_and(|parsed| parsed.to_string() == id && parsed.get_version_num() >= 1
            && parsed.get_version_num() <= 8)
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_timestamp(value: &str) -> bool {
    value.len() <= 48
        && DateTime::parse_from_rfc3339(value).is_ok()
}

impl AgentTaskExecutionRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EXECUTION_SCHEMA_VERSION {
            return invalid("unsupported schema version");
        }
        if !valid_id(&self.execution_id) || !valid_id(&self.task_id) || !valid_id(&self.workspace_id) {
            return invalid("IDs must be canonical lowercase RFC 4122 UUIDs");
        }
        if self.adapter_version.is_empty()
            || self.adapter_version.len() > 128
            || self.adapter_version.chars().any(char::is_control)
        {
            return invalid("invalid adapter version");
        }
        if !valid_timestamp(&self.started_at)
            || self.finished_at.as_deref().is_some_and(|s| !valid_timestamp(s))
        {
            return invalid("timestamps must be bounded RFC3339 values");
        }
        if let Some(finished) = &self.finished_at {
            let started = DateTime::parse_from_rfc3339(&self.started_at)
                .map_err(|_| CoreError::Config("invalid start time".into()))?;
            let finish = DateTime::parse_from_rfc3339(finished)
                .map_err(|_| CoreError::Config("invalid finish time".into()))?;
            if finish < started {
                return invalid("finish precedes start");
            }
        }
        if !valid_sha(&self.task_scope_digest)
            || !valid_sha(&self.prompt_sha256)
            || self.executable_sha256.as_deref().is_some_and(|s| !valid_sha(s))
            || self.output_aggregate_sha256.as_deref().is_some_and(|s| !valid_sha(s))
        {
            return invalid("hashes must be 64 lowercase hex characters");
        }
        if self.enabled_task_tools.len() > TASK_TOOLS.len() {
            return invalid("too many enabled task tools");
        }
        let mut tools = HashSet::new();
        if self.enabled_task_tools.iter().any(|tool| !tools.insert(*tool)) {
            return invalid("duplicate enabled task tool");
        }
        if self.output_file_count > MAX_EXECUTION_FILES
            || self.output_total_bytes > MAX_EXECUTION_BYTES
        {
            return invalid("output counters exceed limits");
        }
        if self.output_file_count == 0 {
            if self.output_total_bytes != 0 || self.output_aggregate_sha256.is_some() {
                return invalid("empty output must have zero bytes and no aggregate hash");
            }
        } else if self.output_aggregate_sha256.is_none() {
            return invalid("nonempty output requires an aggregate hash");
        }
        if let Some(error) = &self.error_code {
            if error.len() > 64
                || !error.starts_with("ERR_")
                || error.len() < 5
                || !error[4..].bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                return invalid("unsafe error code");
            }
        }
        if let Some(provider_id) = &self.provider_execution_id {
            if provider_id.is_empty()
                || provider_id.len() > 128
                || !provider_id.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-')
                })
            {
                return invalid("unsafe provider execution id");
            }
        }
        match self.runtime_kind {
            RuntimeKind::LocalClient => {
                if !matches!(
                    self.adapter_kind,
                    AdapterKind::Codex | AdapterKind::ClaudeCode | AdapterKind::Opencode
                        | AdapterKind::GeminiCli | AdapterKind::Pi
                ) || self.runtime_provider.is_some()
                    || self.executable_sha256.is_none()
                    || self.provider_execution_id.is_some()
                    || self.isolation_profile != IsolationProfile::McpOnlyReadonlyHost
                {
                    return invalid("inconsistent local-client identity and isolation");
                }
            }
            RuntimeKind::RemoteSandbox => {
                let expected = match self.adapter_kind {
                    AdapterKind::OpenaiSandbox => RuntimeProvider::Openai,
                    AdapterKind::Daytona => RuntimeProvider::Daytona,
                    AdapterKind::Modal => RuntimeProvider::Modal,
                    _ => return invalid("remote runtime requires a sandbox adapter"),
                };
                if self.runtime_provider != Some(expected)
                    || self.executable_sha256.is_some()
                    || self.isolation_profile != IsolationProfile::RemoteBundleIsolated
                {
                    return invalid("inconsistent remote-sandbox identity and isolation");
                }
            }
        }
        match self.status {
            ExecutionStatus::Prepared | ExecutionStatus::Running => {
                if self.finished_at.is_some() || self.error_code.is_some()
                    || self.output_file_count != 0
                {
                    return invalid("unfinished execution has final state or outputs");
                }
            }
            ExecutionStatus::Succeeded => {
                if self.finished_at.is_none() || self.error_code.is_some() {
                    return invalid("successful execution must have finish time and no error");
                }
            }
            ExecutionStatus::Failed | ExecutionStatus::Cancelled => {
                if self.finished_at.is_none() || self.error_code.is_none() {
                    return invalid("failed or cancelled execution requires time and safe error");
                }
            }
        }
        Ok(())
    }

    /// Parse untrusted evidence without accepting omitted nullable fields,
    /// duplicate known keys, unknown keys, or oversized JSON.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return invalid("record exceeds 32 KiB");
        }
        let record: Self = serde_json::from_slice(bytes)?;
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let object = value.as_object().ok_or_else(|| {
            CoreError::Config("execution evidence must be a JSON object".into())
        })?;
        if object.len() != REQUIRED_FIELDS.len()
            || REQUIRED_FIELDS.iter().any(|field| !object.contains_key(*field))
        {
            return invalid("all 21 schema fields must be present");
        }
        record.validate()?;
        Ok(record)
    }

    fn immutable_identity_matches(&self, next: &Self) -> bool {
        self.schema_version == next.schema_version
            && self.execution_id == next.execution_id
            && self.task_id == next.task_id
            && self.workspace_id == next.workspace_id
            && self.adapter_kind == next.adapter_kind
            && self.adapter_version == next.adapter_version
            && self.executable_sha256 == next.executable_sha256
            && self.runtime_kind == next.runtime_kind
            && self.runtime_provider == next.runtime_provider
            && self.started_at == next.started_at
            && self.task_scope_digest == next.task_scope_digest
            && self.enabled_task_tools == next.enabled_task_tools
            && self.isolation_profile == next.isolation_profile
            && self.prompt_sha256 == next.prompt_sha256
    }

    fn validate_transition(&self, next: &Self) -> Result<()> {
        self.validate()?;
        next.validate()?;
        if !self.immutable_identity_matches(next) {
            return invalid("execution identity, scope, tools, or launch details changed");
        }
        let allowed = matches!(
            (self.status, next.status),
            (ExecutionStatus::Prepared, ExecutionStatus::Running)
                | (ExecutionStatus::Prepared, ExecutionStatus::Failed)
                | (ExecutionStatus::Prepared, ExecutionStatus::Cancelled)
                | (ExecutionStatus::Running, ExecutionStatus::Succeeded)
                | (ExecutionStatus::Running, ExecutionStatus::Failed)
                | (ExecutionStatus::Running, ExecutionStatus::Cancelled)
        );
        if !allowed {
            return invalid("illegal execution lifecycle transition");
        }
        if self.provider_execution_id.is_some()
            && self.provider_execution_id != next.provider_execution_id
        {
            return invalid("provider execution ID changed after allocation");
        }
        if self.provider_execution_id.is_none()
            && next.provider_execution_id.is_some()
            && self.runtime_kind != RuntimeKind::RemoteSandbox
        {
            return invalid("local execution cannot acquire a provider ID");
        }
        Ok(())
    }
}

pub fn generate_execution_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn private_execution_root(config_dir: &Path) -> PathBuf {
    config_dir.join(PRIVATE_EXECUTIONS_DIR)
}

/// Stores all authoritative records outside of any Agent Task/Workspace tree.
/// Only trusted orchestration should own this handle.
#[derive(Debug, Clone)]
pub struct ExecutionRecordStore {
    root: PathBuf,
}

impl ExecutionRecordStore {
    pub fn new(config_dir: &Path) -> Self {
        Self { root: private_execution_root(config_dir) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn checked_record_path(&self, workspace: &str, task: &str, execution: &str) -> Result<PathBuf> {
        if !valid_id(workspace) || !valid_id(task) || !valid_id(execution) {
            return invalid("invalid execution record path identifier");
        }
        Ok(self.root.join(workspace).join(task).join(format!("{execution}.json")))
    }

    fn ensure_layout(&self) -> Result<()> {
        // Use only an absolute, application-controlled configuration root.
        if !self.root.is_absolute() {
            return invalid("execution state root must be absolute");
        }
        if let Some(config_dir) = self.root.parent() {
            atomic_file::create_dir_all(config_dir)?;
            ensure_real_directory(config_dir)?;
        }
        atomic_file::create_dir_all(&self.root)?;
        ensure_real_directory(&self.root)?;
        Ok(())
    }

    fn lock(&self) -> Result<File> {
        self.ensure_layout()?;
        let path = self.root.join(MUTATION_LOCK);
        reject_link_or_nonfile(&path, true)?;
        let file = open_restricted_file(&path)?;
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == ErrorKind::WouldBlock
                    && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    return invalid("execution state mutation lock timed out");
                }
                Err(error) => return Err(CoreError::Io(error)),
            }
        }
    }

    pub fn create(&self, record: &AgentTaskExecutionRecord) -> Result<()> {
        record.validate()?;
        if record.status != ExecutionStatus::Prepared {
            return invalid("new executions must begin in prepared state");
        }
        let path = self.checked_record_path(
            &record.workspace_id, &record.task_id, &record.execution_id
        )?;
        let _lock = self.lock()?;
        let parent = path.parent().ok_or_else(|| {
            CoreError::Config("execution record has no parent".into())
        })?;
        atomic_file::create_dir_all(parent)?;
        ensure_real_directory(parent)?;
        let bytes = serialize_bounded(record)?;
        // Create through a fully-written temporary file and a no-replace hard link.
        // Unlike an existence check followed by rename, this never overwrites an
        // existing record when another process has already created the same ID.
        atomic_create_no_replace(&path, &bytes)
    }

    pub fn read(&self, workspace: &str, task: &str, execution: &str) -> Result<AgentTaskExecutionRecord> {
        let path = self.checked_record_path(workspace, task, execution)?;
        let record = read_bounded(&path)?;
        if record.workspace_id != workspace || record.task_id != task
            || record.execution_id != execution
        {
            return invalid("execution record is stored under mismatched identifiers");
        }
        Ok(record)
    }

    /// Caller passes the last observed record, not just the status. This rejects
    /// stale writers even when a competing process already advanced the record.
    pub fn transition(
        &self,
        expected: &AgentTaskExecutionRecord,
        next: &AgentTaskExecutionRecord,
    ) -> Result<()> {
        expected.validate_transition(next)?;
        let _lock = self.lock()?;
        let on_disk = self.read(&expected.workspace_id, &expected.task_id, &expected.execution_id)?;
        if on_disk != *expected {
            return invalid("stale execution evidence revision; reload and retry");
        }
        let path = self.checked_record_path(
            &expected.workspace_id, &expected.task_id, &expected.execution_id
        )?;
        reject_link_or_nonfile(&path, false)?;
        let bytes = serialize_bounded(next)?;
        atomic_file::atomic_write_file(&path, &bytes, FILE_MODE)
    }
}

fn serialize_bounded(record: &AgentTaskExecutionRecord) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(record)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return invalid("serialized execution record exceeds 32 KiB");
    }
    Ok(bytes)
}

fn ensure_real_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || is_reparse_point(&meta) {
        return invalid("private execution state directory is not a real directory");
    }
    Ok(())
}

fn reject_link_or_nonfile(path: &Path, allow_missing: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && !is_reparse_point(&meta) => Ok(()),
        Ok(_) => invalid("execution evidence path must be a regular file"),
        Err(error) if allow_missing && error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::Io(error)),
    }
}

#[cfg(windows)]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x0400 != 0
}
#[cfg(not(windows))]
fn is_reparse_point(_meta: &fs::Metadata) -> bool {
    false
}

fn open_restricted_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    Ok(options.open(path)?)
}

fn read_bounded(path: &Path) -> Result<AgentTaskExecutionRecord> {
    reject_link_or_nonfile(path, false)?;
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
    AgentTaskExecutionRecord::parse(&bytes)
}

/// Atomically expose a complete file only if its name is still unused.
/// The temporary file and final record share a filesystem.
fn atomic_create_no_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        CoreError::Config("execution record has no parent".into())
    })?;
    let temp = parent.join(format!(".execution-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let mut file = options.open(&temp)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    drop(file);
    let _ = fs::remove_file(&temp);
    if let Err(error) = result {
        if error.kind() == ErrorKind::AlreadyExists {
            return invalid("execution ID already exists");
        }
        return Err(CoreError::Io(error));
    }
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}
