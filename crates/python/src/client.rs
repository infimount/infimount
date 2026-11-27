use std::cell::Cell;
use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infimount_core::operations;
use infimount_mcp::audit::{AuditDecision, AuditEvent};
use infimount_mcp::storage_namespace::validate_local_mcp_path_for_operation;
use infimount_mcp::{
    evaluate_storage_policy, parse_mcp_path, McpOperation, PolicyDecision, StorageRecord,
};
use opendal::Operator;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;

use crate::config::{canonical_relative, Config, Limits, Scope};
use crate::error::{Error, Result};

const AUDIT_CAPACITY: usize = 256;

#[derive(Clone, Copy)]
pub enum Operation {
    List,
    Stat,
    Read,
    Write,
    Delete,
    DownloadLink,
}

impl Operation {
    fn policy(self) -> McpOperation {
        match self {
            Self::List => McpOperation::List,
            Self::Stat => McpOperation::Metadata,
            Self::Read => McpOperation::Read,
            Self::Write => McpOperation::Write,
            Self::Delete => McpOperation::Delete,
            Self::DownloadLink => McpOperation::PresignDownloadLink,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::List => "list_dir",
            Self::Stat => "stat_path",
            Self::Read => "read_file",
            Self::Write => "write_file",
            Self::Delete => "delete_file",
            Self::DownloadLink => "generate_download_link",
        }
    }
}

struct OwnedRuntime {
    runtime: Option<Runtime>,
    pid: u32,
}

impl Drop for OwnedRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            if self.pid != std::process::id() {
                // A fork inherited runtime bookkeeping but not its worker
                // threads. Never wait for those vanished workers in the child.
                std::mem::forget(runtime);
            } else {
                runtime.shutdown_background();
            }
        }
    }
}

pub struct Client {
    record: StorageRecord,
    op: Operator,
    scopes: BTreeSet<Scope>,
    pub limits: Limits,
    runtime: OwnedRuntime,
    permits: Arc<Semaphore>,
    closed: AtomicBool,
    audit: Mutex<VecDeque<AuditEvent>>,
    audit_dropped: AtomicU64,
}

impl Client {
    pub fn new(text: &str) -> Result<Self> {
        let config = Config::parse(text)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(4)
            .enable_all()
            .thread_name("infimount-python")
            .build()
            .map_err(|_| Error::Unavailable)?;
        let (record, op, scopes, limits) = {
            let _entered = runtime.enter();
            config.build()?
        };
        Ok(Self {
            record,
            op,
            scopes,
            limits,
            runtime: OwnedRuntime {
                runtime: Some(runtime),
                pid: std::process::id(),
            },
            permits: Arc::new(Semaphore::new(limits.concurrent_operations)),
            closed: AtomicBool::new(false),
            audit: Mutex::new(VecDeque::with_capacity(AUDIT_CAPACITY)),
            audit_dropped: AtomicU64::new(0),
        })
    }

    fn active(&self) -> Result<()> {
        if self.runtime.pid != std::process::id() {
            return Err(Error::Forked);
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        Ok(())
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub fn description(&self) -> String {
        format!(
            "<infimount.Client backend={} closed={}>",
            self.record.backend,
            self.closed.load(Ordering::Acquire)
        )
    }

    fn authorize_relative(&self, relative: &str, operation: Operation) -> Result<()> {
        if !canonical_relative(relative, true) {
            return Err(Error::InvalidRequest);
        }
        let scope = match operation {
            Operation::Write => Scope::Create,
            Operation::Delete => Scope::Delete,
            _ => Scope::Read,
        };
        if !self.scopes.contains(&scope)
            || (matches!(operation, Operation::DownloadLink)
                && !self.scopes.contains(&Scope::Download))
        {
            return Err(Error::PermissionDenied);
        }
        if !self.record.enabled || !self.record.mcp_exposed {
            return Err(Error::PermissionDenied);
        }
        for policy_op in [
            Some(operation.policy()),
            matches!(operation, Operation::DownloadLink).then_some(McpOperation::Read),
        ]
        .into_iter()
        .flatten()
        {
            let evaluation =
                evaluate_storage_policy(&self.record, relative, policy_op, false, false)?;
            if !matches!(evaluation.decision, PolicyDecision::Allow) {
                return Err(Error::ConfirmationRequired);
            }
            validate_local_mcp_path_for_operation(&self.record, relative, Some(policy_op))?;
        }
        crate::local::verify_spelling(&self.record, relative)?;
        Ok(())
    }

    fn authorize(&self, path: &str, operation: Operation) -> Result<String> {
        if path.len() > 4096
            || path.trim() != path
            || path.contains(['\\', '%', ':'])
            || path.chars().any(char::is_control)
        {
            return Err(Error::InvalidRequest);
        }
        let parsed = parse_mcp_path(path)?;
        if parsed.normalized != path.trim_end_matches('/')
            || parsed.storage_name.as_deref() != Some(self.record.name.as_str())
        {
            return Err(Error::PermissionDenied);
        }
        if parsed.backend_path.is_empty() && !matches!(operation, Operation::List | Operation::Stat)
        {
            return Err(Error::InvalidRequest);
        }
        self.authorize_relative(&parsed.backend_path, operation)?;
        let capability = self.op.info().capability();
        let supported = match operation {
            Operation::List => capability.list,
            Operation::Stat => capability.stat,
            Operation::Read => capability.stat && capability.read,
            Operation::Write => capability.write && capability.write_with_if_not_exists,
            Operation::Delete => capability.stat && capability.delete,
            Operation::DownloadLink => capability.stat && capability.presign_read,
        };
        if !supported {
            return Err(Error::Unsupported);
        }
        Ok(parsed.backend_path)
    }

    fn run<T, F, Fut>(&self, operation: Operation, path: &str, work: F) -> Result<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<(T, u64)>>,
    {
        // Inherited mutexes must not be touched after fork.
        if self.runtime.pid != std::process::id() {
            return Err(Error::Forked);
        }
        let started = Instant::now();
        let result = (|| {
            self.active()?;
            let _permit = self
                .permits
                .clone()
                .try_acquire_owned()
                .map_err(|_| Error::LimitExceeded)?;
            let relative = self.authorize(path, operation)?;
            if tokio::runtime::Handle::try_current().is_ok() {
                return Err(Error::Unsupported);
            }
            self.runtime
                .runtime
                .as_ref()
                .ok_or(Error::Closed)?
                .block_on(work(relative))
        })();
        self.record(operation, path, started, &result);
        result.map(|(value, _)| value)
    }

    fn record<T>(
        &self,
        operation: Operation,
        path: &str,
        started: Instant,
        result: &Result<(T, u64)>,
    ) {
        let mut event = AuditEvent::new(operation.name(), operation.policy());
        event.actor_type = "python".into();
        // The desktop activation-probe environment is not authority or audit
        // identity for an embedded application.
        event.correlation_id = None;
        event.storage_id = Some(self.record.id.clone());
        event.backend = Some(self.record.backend.clone());
        event.path = Some(format!("sha256:{:x}", Sha256::digest(path.as_bytes())));
        event.duration_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        event.decision = match result {
            Ok(_) => AuditDecision::Allowed,
            Err(Error::ConfirmationRequired) => AuditDecision::RequiresConfirmation,
            Err(
                Error::PermissionDenied
                | Error::InvalidRequest
                | Error::Unsupported
                | Error::LimitExceeded,
            ) => AuditDecision::Denied,
            Err(_) => AuditDecision::Failed,
        };
        if let Err(error) = result {
            event.error_code = Some(error.code().into());
        }
        if let Ok((_, bytes)) = result {
            if matches!(operation, Operation::Read) {
                event.bytes_read = Some(*bytes);
            }
            if matches!(operation, Operation::Write) {
                event.bytes_written = Some(*bytes);
            }
        }
        if let Ok(serialized) = serde_json::to_string(&event) {
            tracing::info!(target: "infimount_python::audit", event = %serialized);
        }
        if let Ok(mut audit) = self.audit.lock() {
            if audit.len() == AUDIT_CAPACITY {
                audit.pop_front();
                self.audit_dropped.fetch_add(1, Ordering::Relaxed);
            }
            audit.push_back(event);
        }
    }

    pub fn reject_oversized_write(&self, path: &str) -> Error {
        if self.runtime.pid != std::process::id() {
            return Error::Forked;
        }
        let error = Error::LimitExceeded;
        self.record::<()>(Operation::Write, path, Instant::now(), &Err(error));
        error
    }

    pub fn audit_json(&self) -> Result<String> {
        if self.runtime.pid != std::process::id() {
            return Err(Error::Forked);
        }
        let mut audit = self.audit.lock().map_err(|_| Error::Unavailable)?;
        let events: Vec<_> = audit.drain(..).collect();
        serde_json::to_string(
            &json!({"events":events,"dropped":self.audit_dropped.swap(0, Ordering::AcqRel)}),
        )
        .map_err(|_| Error::Unavailable)
    }

    pub fn list(&self, path: &str, limit: u32, cursor: Option<String>) -> Result<String> {
        self.run(Operation::List, path, |relative| async move {
            if !(1..=200).contains(&limit) || cursor.as_ref().is_some_and(|value| value.len() > 4096) {
                return Err(Error::LimitExceeded);
            }
            let filter_error = Cell::new(None);
            let page = operations::list_entries_page_with_filter(
                &self.op, &relative, limit, cursor, false, self.record.revision,
                |entry| match self.authorize_relative(entry.trim_matches('/'), Operation::List) {
                    Ok(()) => Ok(true),
                    Err(Error::PermissionDenied | Error::ConfirmationRequired | Error::InvalidRequest | Error::NotFound) => Ok(false),
                    Err(error) => {
                        // Policy-hidden entries are expected. A resource or I/O
                        // failure must not become an apparently complete page.
                        filter_error.set(Some(error));
                        Err(infimount_core::CoreError::Config("listing validation failed".into()))
                    }
                },
            ).await;
            if let Some(error) = filter_error.get() {
                return Err(error);
            }
            let page = page?;
            let entries: Vec<_> = page.entries.into_iter()
                .filter(|entry| entry.path.trim_matches('/') != relative)
                .map(|mut entry| {
                    entry.path = format!("/{}/{}", self.record.name, entry.path.trim_matches('/'));
                    entry
                }).collect();
            let value = json!({"entries":entries,"next_cursor":page.next_cursor,"truncated":page.truncated});
            Ok((value.to_string(), 0))
        })
    }

    pub fn stat(&self, path: &str) -> Result<String> {
        self.run(Operation::Stat, path, |relative| async move {
            let mut entry = operations::stat_entry(&self.op, &relative).await?;
            entry.path = path.trim_end_matches('/').into();
            Ok((
                serde_json::to_string(&entry).map_err(|_| Error::Unavailable)?,
                0,
            ))
        })
    }

    pub fn read(&self, path: &str, offset: u64, max_bytes: Option<u64>) -> Result<Vec<u8>> {
        self.run(Operation::Read, path, |relative| async move {
            let maximum = max_bytes.unwrap_or(self.limits.read_bytes as u64);
            if maximum == 0 || maximum > self.limits.read_bytes as u64 {
                return Err(Error::LimitExceeded);
            }
            let meta = operations::stat_entry(&self.op, &relative).await?;
            if meta.is_dir || offset > meta.size {
                return Err(Error::InvalidRequest);
            }
            if max_bytes.is_none() && meta.size - offset > maximum {
                return Err(Error::LimitExceeded);
            }
            let end = offset.saturating_add(maximum).min(meta.size);
            let read =
                operations::read_range_bounded(&self.op, &relative, offset, end, None).await?;
            self.authorize_relative(&relative, Operation::Read)?;
            let bytes = read.len() as u64;
            Ok((read, bytes))
        })
    }

    pub fn write(&self, path: &str, bytes: Vec<u8>) -> Result<u64> {
        self.run(Operation::Write, path, |relative| async move {
            if bytes.len() > self.limits.write_bytes {
                return Err(Error::LimitExceeded);
            }
            if self.record.backend == "local" {
                if let Some((parent, _)) = relative.rsplit_once('/') {
                    let parent = operations::stat_entry(&self.op, parent).await?;
                    if !parent.is_dir {
                        return Err(Error::InvalidRequest);
                    }
                }
            }
            self.authorize_relative(&relative, Operation::Write)?;
            let size = bytes.len() as u64;
            // Exactly the same backend precondition used by MCP create-only
            // writes. Never substitute stat-then-overwrite on unsupported stores.
            operations::write_full_with_options(&self.op, &relative, bytes, false, None, None)
                .await?;
            Ok((size, size))
        })
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        self.run(Operation::Delete, path, |relative| async move {
            let entry = operations::stat_entry(&self.op, &relative).await?;
            if entry.is_dir {
                return Err(Error::Unsupported);
            }
            self.authorize_relative(&relative, Operation::Delete)?;
            // A single-object delete cannot recurse into newly introduced or
            // denied descendants, even if the object changes after stat.
            self.op.delete(&relative).await?;
            Ok(((), 0))
        })
    }

    pub fn download_link(&self, path: &str, expires_seconds: u64) -> Result<String> {
        self.run(Operation::DownloadLink, path, |relative| async move {
            if !(60..=900).contains(&expires_seconds) { return Err(Error::LimitExceeded); }
            let entry = operations::stat_entry(&self.op, &relative).await?;
            if entry.is_dir { return Err(Error::InvalidRequest); }
            self.authorize_relative(&relative, Operation::DownloadLink)?;
            let request = self.op.presign_read(&relative, Duration::from_secs(expires_seconds)).await?;
            let url = request.uri().to_string();
            // Presigned provider URLs are intentionally returned only by this
            // explicitly authorized operation, never included in audit/error data.
            let headers: std::collections::BTreeMap<_, _> = request.header().iter()
                .map(|(name, value)| Ok((name.to_string(), value.to_str().map_err(|_| Error::Unavailable)?.to_owned())))
                .collect::<Result<_>>()?;
            Ok((json!({"url":url,"method":"GET","headers":headers,"expires_seconds":expires_seconds}).to_string(), 0))
        })
    }
}
