//! Owner-bound, single-use transfer grants and bounded private staging.
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures::{io::AsyncReadExt, StreamExt};
use infimount_core::hosted_io::{PinnedReader, StagedFile, STREAM_BUFFER_BYTES};
use infimount_mcp::McpOperation;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Principal;
use crate::delegation::DelegatedIssuer;
use crate::error::{HostedError, Result};
use crate::storage::{
    map_core, validate_content_type, AuthorizedObject, ObjectIdentity, StoragePlane,
};

#[derive(Clone)]
pub struct TransferManager {
    inner: Arc<Inner>,
}

struct Inner {
    storage: Arc<StoragePlane>,
    issuer: Option<DelegatedIssuer>,
    entries: Mutex<HashMap<String, Arc<GrantEntry>>>,
    budget: Arc<Mutex<Budget>>,
}

#[derive(Default)]
struct Budget {
    bytes: usize,
    count: usize,
    owners: HashMap<String, (usize, usize)>,
}

struct Reservation {
    owner: String,
    bytes: usize,
    budget: Arc<Mutex<Budget>>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut budget) = self.budget.lock() {
            budget.bytes = budget.bytes.saturating_sub(self.bytes);
            budget.count = budget.count.saturating_sub(1);
            if let Some(owner) = budget.owners.get_mut(&self.owner) {
                owner.0 = owner.0.saturating_sub(self.bytes);
                owner.1 = owner.1.saturating_sub(1);
                if owner.1 == 0 {
                    budget.owners.remove(&self.owner);
                }
            }
        }
    }
}

struct GrantEntry {
    owner: String,
    expires: Instant,
    expires_epoch: u64,
    object: AuthorizedObject,
    state: Mutex<GrantState>,
    _reservation: Arc<Reservation>,
}

impl GrantEntry {
    fn active(&self) -> bool {
        self.expires > Instant::now() && crate::now_epoch_seconds() < self.expires_epoch
    }
}

struct DelegationAudit<'a> {
    principal: &'a Principal,
    path: &'a str,
    completed: bool,
}

impl DelegationAudit<'_> {
    fn finish(&mut self, result: Result<()>) {
        self.completed = true;
        crate::audit::record(
            Some(self.principal),
            "delegate_download",
            result,
            Some(self.path),
            0,
        );
    }
}

impl Drop for DelegationAudit<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.finish(Err(HostedError::Unavailable));
        }
    }
}

enum GrantState {
    Upload {
        stage: Option<StagedFile>,
        sha256: String,
        content_type: Option<String>,
        ready: bool,
    },
    Download {
        stage: Option<StagedFile>,
        identity: ObjectIdentity,
    },
    Claimed,
}

/// If a future is cancelled or a connection disappears after its grant was
/// claimed, this drops the private stage and removes the map entry. A racing
/// request cannot claim a second writer or retain an unaccounted stage.
struct UploadClaim {
    manager: Weak<Inner>,
    id: String,
    entry: Arc<GrantEntry>,
    stage: Option<StagedFile>,
    retained: bool,
}

impl Drop for UploadClaim {
    fn drop(&mut self) {
        if !self.retained {
            if let Some(manager) = self.manager.upgrade() {
                if let Ok(mut entries) = manager.entries.lock() {
                    entries.remove(&self.id);
                }
            }
        }
    }
}

impl TransferManager {
    pub fn new(storage: Arc<StoragePlane>) -> Self {
        Self::with_issuer(storage, None)
    }

    pub fn with_issuer(storage: Arc<StoragePlane>, issuer: Option<DelegatedIssuer>) -> Self {
        let manager = Self {
            inner: Arc::new(Inner {
                storage,
                issuer,
                entries: Mutex::new(HashMap::new()),
                budget: Arc::new(Mutex::new(Budget::default())),
            }),
        };
        let weak = Arc::downgrade(&manager.inner);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    if let Ok(mut entries) = inner.entries.lock() {
                        entries.retain(|_, entry| entry.active());
                    };
                }
            });
        }
        manager
    }

    /// Network inline writes share the pending-transfer staging budget.
    pub async fn write_file(
        &self,
        principal: &Principal,
        path: &str,
        content: &str,
        encoding: &str,
        content_type: Option<&str>,
    ) -> Result<Value> {
        let cap = self.inner.storage.config().limits.inline_bytes;
        let size = match encoding {
            "utf-8" => content.len(),
            "base64" => {
                if !content.len().is_multiple_of(4) || content.len() > cap.div_ceil(3) * 4 {
                    return Err(HostedError::Limit);
                }
                let padding = if content.ends_with("==") {
                    2
                } else if content.ends_with('=') {
                    1
                } else {
                    0
                };
                (content.len() / 4 * 3).saturating_sub(padding)
            }
            _ => return Err(HostedError::InvalidRequest),
        };
        if size > cap {
            return Err(HostedError::Limit);
        }
        self.inner
            .storage
            .authorize(principal, path, McpOperation::Write)?;
        let reservation = Arc::new(self.reserve(principal, size as u64)?);
        let retention = Arc::new((reservation, crate::transport::current_resource_lease()));
        self.inner
            .storage
            .write_with_retention(
                principal,
                path,
                content,
                encoding,
                content_type,
                Some(retention),
            )
            .await
    }

    fn reserve(&self, principal: &Principal, size: u64) -> Result<Reservation> {
        let limits = &self.inner.storage.config().limits;
        let bytes = usize::try_from(size).map_err(|_| HostedError::Limit)?;
        if bytes > limits.transfer_bytes {
            return Err(HostedError::Limit);
        }
        {
            let mut entries = self
                .inner
                .entries
                .lock()
                .map_err(|_| HostedError::Internal)?;
            entries.retain(|_, entry| entry.active());
        }
        let owner = principal.owner_key();
        let mut budget = self
            .inner
            .budget
            .lock()
            .map_err(|_| HostedError::Internal)?;
        let used = budget.owners.get(&owner).copied().unwrap_or_default();
        if budget.count >= limits.max_grants
            || used.1 >= limits.max_grants_per_principal
            || budget.bytes.saturating_add(bytes) > limits.staging_bytes
            || used.0.saturating_add(bytes) > limits.principal_staging_bytes
        {
            return Err(HostedError::Limit);
        }
        budget.count += 1;
        budget.bytes += bytes;
        budget
            .owners
            .insert(owner.clone(), (used.0 + bytes, used.1 + 1));
        Ok(Reservation {
            owner,
            bytes,
            budget: self.inner.budget.clone(),
        })
    }

    fn expiry(&self, principal: &Principal, ttl: Option<u64>) -> Result<(Instant, u64)> {
        principal.ensure_live()?;
        let max = self.inner.storage.config().limits.grant_ttl_seconds;
        let ttl = ttl.unwrap_or(max);
        if ttl == 0 || ttl > max {
            return Err(HostedError::InvalidRequest);
        }
        // Derive both clocks from one absolute deadline. Adding the rounded
        // token lifetime to Instant::now() could otherwise extend authority
        // beyond exp by the current fractional second.
        let monotonic_now = Instant::now();
        let now = std::time::SystemTime::now();
        let expires_epoch = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| HostedError::Internal)?
            .as_secs()
            .saturating_add(ttl)
            .min(principal.expires_at);
        let remaining = (std::time::UNIX_EPOCH + Duration::from_secs(expires_epoch))
            .duration_since(now)
            .map_err(|_| HostedError::Unauthorized)?;
        if remaining.is_zero() {
            return Err(HostedError::Unauthorized);
        }
        Ok((monotonic_now + remaining, expires_epoch))
    }

    fn lookup(&self, principal: &Principal, id: &str) -> Result<Arc<GrantEntry>> {
        principal.ensure_live()?;
        if Uuid::parse_str(id).is_err() {
            return Err(HostedError::NotFound);
        }
        let mut entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| HostedError::Internal)?;
        entries.retain(|_, entry| entry.active());
        let entry = entries.get(id).ok_or(HostedError::NotFound)?;
        if entry.owner != principal.owner_key() {
            return Err(HostedError::NotFound);
        }
        Ok(entry.clone())
    }

    fn insert(&self, entry: GrantEntry) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        self.insert_with_id(id.clone(), entry)?;
        Ok(id)
    }

    fn insert_with_id(&self, id: String, entry: GrantEntry) -> Result<()> {
        if !entry.active() {
            return Err(HostedError::NotFound);
        }
        let mut entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| HostedError::Internal)?;
        match entries.entry(id) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(Arc::new(entry));
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(_) => Err(HostedError::Internal),
        }
    }

    pub async fn create_upload(
        &self,
        principal: &Principal,
        path: &str,
        size: u64,
        sha256: &str,
        content_type: Option<&str>,
        ttl_seconds: Option<u64>,
    ) -> Result<Value> {
        validate_content_type(content_type)?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(HostedError::InvalidRequest);
        }
        self.inner
            .storage
            .authorize(principal, path, McpOperation::Write)?;
        let (expires, expires_epoch) = self.expiry(principal, ttl_seconds)?;
        let reservation = Arc::new(self.reserve(principal, size)?);
        let (object, mut stage) =
            self.inner
                .storage
                .new_stage(principal, path, size, content_type)?;
        stage.set_resource_retention(Some(reservation.clone()));
        let id = self.insert(GrantEntry {
            owner: principal.owner_key(),
            expires,
            expires_epoch,
            object,
            state: Mutex::new(GrantState::Upload {
                stage: Some(stage),
                sha256: sha256.into(),
                content_type: content_type.map(str::to_owned),
                ready: false,
            }),
            _reservation: reservation,
        })?;
        Ok(
            json!({"upload_id":id,"url":format!("{}/uploads/{id}",self.inner.storage.config().public_url.trim_end_matches('/')),
            "method":"PUT","requires_authorization":true,
            "expires_seconds":expires_epoch.saturating_sub(crate::now_epoch_seconds()),
            "expires_at_unix_seconds":expires_epoch,"size_bytes":size,"sha256":sha256,"state":"created"}),
        )
    }

    pub async fn upload(&self, principal: &Principal, id: &str, body: Body) -> Result<Value> {
        let entry = self.lookup(principal, id)?;
        self.inner
            .storage
            .authorize(principal, &entry.object.canonical, McpOperation::Write)?;
        let (stage, sha256, content_type) = {
            let mut state = entry.state.lock().map_err(|_| HostedError::Internal)?;
            match &mut *state {
                GrantState::Upload {
                    stage,
                    sha256,
                    content_type,
                    ready: false,
                } => {
                    let tuple = (
                        stage.take().ok_or(HostedError::Conflict)?,
                        sha256.clone(),
                        content_type.clone(),
                    );
                    *state = GrantState::Claimed;
                    tuple
                }
                _ => return Err(HostedError::Conflict),
            }
        };
        let mut claim = UploadClaim {
            manager: Arc::downgrade(&self.inner),
            id: id.into(),
            entry,
            stage: Some(stage),
            retained: false,
        };
        claim
            .stage
            .as_mut()
            .ok_or(HostedError::Internal)?
            .set_resource_retention(Some(Arc::new((
                claim.entry._reservation.clone(),
                crate::transport::current_resource_lease(),
            ))));
        let result = async {
            let mut stream = body.into_data_stream();
            let deadline = claim.entry.expires.min(
                Instant::now()
                    + Duration::from_secs(
                        self.inner.storage.config().limits.request_timeout_seconds,
                    ),
            );
            while let Some(frame) =
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), stream.next())
                    .await
                    .map_err(|_| HostedError::Unavailable)?
            {
                principal.ensure_live()?;
                self.inner.storage.authorize(
                    principal,
                    &claim.entry.object.canonical,
                    McpOperation::Write,
                )?;
                let bytes = frame.map_err(|_| HostedError::InvalidRequest)?;
                // Frames may be larger than configured chunks. Slice them
                // without making another whole-frame allocation.
                for chunk in bytes.chunks(
                    self.inner
                        .storage
                        .config()
                        .limits
                        .chunk_bytes
                        .min(STREAM_BUFFER_BYTES),
                ) {
                    if !claim.entry.active() || Instant::now() >= deadline {
                        return Err(HostedError::Unavailable);
                    }
                    claim
                        .stage
                        .as_mut()
                        .ok_or(HostedError::Internal)?
                        .write_chunk(chunk)
                        .await
                        .map_err(map_core)?;
                }
            }
            claim
                .stage
                .as_mut()
                .ok_or(HostedError::Internal)?
                .finish(Some(&sha256))
                .await
                .map_err(map_core)?;
            principal.ensure_live()?;
            if !claim.entry.active() {
                return Err(HostedError::NotFound);
            }
            let bytes = claim.stage.as_ref().ok_or(HostedError::Internal)?.size();
            // Pending ready uploads keep quota but release the HTTP request
            // permits. No stage owns its GrantEntry, so there is no cycle.
            claim
                .stage
                .as_mut()
                .ok_or(HostedError::Internal)?
                .set_resource_retention(Some(claim.entry._reservation.clone()));
            let mut state = claim
                .entry
                .state
                .lock()
                .map_err(|_| HostedError::Internal)?;
            *state = GrantState::Upload {
                stage: claim.stage.take(),
                sha256: sha256.clone(),
                content_type,
                ready: true,
            };
            claim.retained = true;
            Ok(json!({"upload_id":id,"state":"ready","received_bytes":bytes,"sha256":sha256}))
        }
        .await;
        crate::audit::record(
            Some(principal),
            "artifact_upload",
            result.as_ref().map(|_| ()).map_err(|error| *error),
            Some(&claim.entry.object.canonical),
            result
                .as_ref()
                .ok()
                .and_then(|value| value["received_bytes"].as_u64())
                .unwrap_or(0),
        );
        result
    }

    pub async fn complete_upload(&self, principal: &Principal, id: &str) -> Result<Value> {
        let entry = self.lookup(principal, id)?;
        self.inner
            .storage
            .authorize(principal, &entry.object.canonical, McpOperation::Write)?;
        let (stage, sha256, content_type) = {
            let mut state = entry.state.lock().map_err(|_| HostedError::Internal)?;
            match &mut *state {
                GrantState::Upload {
                    stage,
                    sha256,
                    content_type,
                    ready: true,
                } => {
                    let tuple = (
                        stage.take().ok_or(HostedError::Conflict)?,
                        sha256.clone(),
                        content_type.clone(),
                    );
                    *state = GrantState::Claimed;
                    tuple
                }
                _ => return Err(HostedError::Conflict),
            }
        };
        let mut claim = UploadClaim {
            manager: Arc::downgrade(&self.inner),
            id: id.into(),
            entry,
            stage: Some(stage),
            retained: false,
        };
        if !claim.entry.active() {
            return Err(HostedError::NotFound);
        }
        // Storage rechecks authority during publication. A refreshed token
        // must not replace the deadline of the token that issued this grant.
        let commit_principal = Principal {
            expires_at: principal.expires_at.min(claim.entry.expires_epoch),
            ..principal.clone()
        };
        let bytes = claim.stage.as_ref().ok_or(HostedError::Internal)?.size();
        let result = self
            .inner
            .storage
            .commit_stage(
                &commit_principal,
                &claim.entry.object,
                claim.stage.as_mut().ok_or(HostedError::Internal)?,
                content_type.as_deref(),
                Some(claim.entry.expires),
                Some(Arc::new((
                    claim.entry.clone(),
                    crate::transport::current_resource_lease(),
                ))),
            )
            .await;
        crate::audit::record(
            Some(principal),
            "artifact_commit",
            result,
            Some(&claim.entry.object.canonical),
            if result.is_ok() { bytes } else { 0 },
        );
        result?;
        Ok(
            json!({"upload_id":id,"state":"committed","path":claim.entry.object.canonical,"written_bytes":bytes,"sha256":sha256,"content_type":content_type,"overwrite":false}),
        )
    }

    pub async fn abort_upload(&self, principal: &Principal, id: &str) -> Result<Value> {
        let entry = self.lookup(principal, id)?;
        self.inner
            .storage
            .authorize(principal, &entry.object.canonical, McpOperation::Write)?;
        {
            let mut state = entry.state.lock().map_err(|_| HostedError::Internal)?;
            if !matches!(*state, GrantState::Upload { .. }) {
                return Err(HostedError::Conflict);
            }
            *state = GrantState::Claimed;
        }
        self.inner
            .entries
            .lock()
            .map_err(|_| HostedError::Internal)?
            .remove(id);
        crate::audit::record(
            Some(principal),
            "artifact_abort",
            Ok(()),
            Some(&entry.object.canonical),
            0,
        );
        Ok(json!({"upload_id":id,"state":"aborted"}))
    }

    pub async fn create_download(
        &self,
        principal: &Principal,
        path: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<Value> {
        let (expires, expires_epoch) = self.expiry(principal, ttl_seconds)?;
        let source = self
            .inner
            .storage
            .prepare_read(principal, path, true)
            .await?;
        let reservation = Arc::new(self.reserve(principal, source.identity.size)?);
        let mut stage = StagedFile::temporary(source.identity.size).map_err(map_core)?;
        stage.set_resource_retention(Some(Arc::new((
            reservation.clone(),
            crate::transport::current_resource_lease(),
        ))));
        let mut reader = source.reader(0, source.identity.size).await?;
        let deadline = expires.min(
            Instant::now()
                + Duration::from_secs(self.inner.storage.config().limits.request_timeout_seconds),
        );
        let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
        loop {
            principal.ensure_live()?;
            self.inner
                .storage
                .authorize(principal, path, McpOperation::Read)?;
            self.inner
                .storage
                .authorize(principal, path, McpOperation::PresignDownloadLink)?;
            let count = tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                reader.read(&mut buffer),
            )
            .await
            .map_err(|_| HostedError::Unavailable)?
            .map_err(|_| HostedError::Unavailable)?;
            if count == 0 {
                break;
            }
            stage
                .write_chunk(&buffer[..count])
                .await
                .map_err(map_core)?;
        }
        stage.finish(None).await.map_err(map_core)?;
        self.inner
            .storage
            .check_identity(principal, &source.object, &source.identity, true)
            .await?;
        if Instant::now() >= expires || crate::now_epoch_seconds() >= expires_epoch {
            return Err(HostedError::NotFound);
        }
        let sha256 = stage.sha256().ok_or(HostedError::Internal)?.to_owned();
        let size = stage.size();
        stage.set_resource_retention(Some(reservation.clone()));
        let id = Uuid::new_v4().to_string();
        let redeem_url = format!("{}/downloads/{id}", self.inner.storage.config().public_url);
        let (url, delivery) = if let Some(issuer) = &self.inner.issuer {
            // All state remains local to this future until validation ends.
            // A failed/cancelled request drops its snapshot and quota without
            // ever publishing a redeemable ID, even if the issuer saw the ID.
            let mut audit = DelegationAudit {
                principal,
                path,
                completed: false,
            };
            let result = async {
                let url = issuer
                    .issue(&id, &redeem_url, expires_epoch, expires)
                    .await?;
                self.inner
                    .storage
                    .check_identity(principal, &source.object, &source.identity, true)
                    .await?;
                principal.ensure_live()?;
                if Instant::now() >= expires || crate::now_epoch_seconds() >= expires_epoch {
                    return Err(HostedError::NotFound);
                }
                Ok(url)
            }
            .await;
            audit.finish(result.as_ref().map(|_| ()).map_err(|error| *error));
            (result?, "delegated")
        } else {
            (redeem_url.clone(), "proxy")
        };
        self.insert_with_id(
            id.clone(),
            GrantEntry {
                owner: principal.owner_key(),
                expires,
                expires_epoch,
                object: source.object,
                state: Mutex::new(GrantState::Download {
                    stage: Some(stage),
                    identity: source.identity,
                }),
                _reservation: reservation,
            },
        )?;
        let mut result = json!({"download_id":id,"url":url,"delivery":delivery,
            "requires_authorization":true,"single_use":true,
            "expires_seconds":expires_epoch.saturating_sub(crate::now_epoch_seconds()),
            "expires_at_unix_seconds":expires_epoch,"size_bytes":size,"sha256":sha256,"content_type":"application/octet-stream"});
        if self.inner.issuer.is_some() {
            result["redeem_url"] = json!(redeem_url);
            result["authorization_audience"] = json!(self.inner.storage.config().auth.audience);
        }
        Ok(result)
    }

    pub async fn download(
        &self,
        principal: &Principal,
        id: &str,
        range: Option<&str>,
    ) -> Result<Response> {
        let entry = self.lookup(principal, id)?;
        let (identity, size) = {
            let state = entry.state.lock().map_err(|_| HostedError::Internal)?;
            match &*state {
                GrantState::Download {
                    stage: Some(stage),
                    identity,
                } => (identity.clone(), stage.size()),
                _ => return Err(HostedError::Conflict),
            }
        };
        let (start, end, partial) = parse_range(range, size)?;
        self.inner
            .storage
            .check_identity(principal, &entry.object, &identity, true)
            .await?;
        let stage = {
            let mut state = entry.state.lock().map_err(|_| HostedError::Internal)?;
            match &mut *state {
                GrantState::Download { stage, .. } => {
                    let stage = stage.take().ok_or(HostedError::Conflict)?;
                    *state = GrantState::Claimed;
                    stage
                }
                _ => return Err(HostedError::Conflict),
            }
        };
        // Remove the single-use ID now. The body retains the reservation until
        // the last snapshot byte is released, including dropped connections.
        self.inner
            .entries
            .lock()
            .map_err(|_| HostedError::Internal)?
            .remove(id);
        let reader = stage
            .reader_with_retention(start, end, crate::transport::current_resource_lease())
            .map_err(map_core)?;
        let state = DownloadState {
            reader,
            _stage: stage,
            entry,
            storage: self.inner.storage.clone(),
            principal: principal.clone(),
            remaining: end - start,
            sent: 0,
            finished: false,
            deadline: Instant::now()
                + Duration::from_secs(self.inner.storage.config().limits.request_timeout_seconds),
        };
        let stream = futures::stream::try_unfold(state, |mut state| async move {
            if state.remaining == 0 {
                if !state.finished {
                    state.finished = true;
                    crate::audit::record(
                        Some(&state.principal),
                        "artifact_download",
                        Ok(()),
                        Some(&state.entry.object.canonical),
                        state.sent,
                    );
                }
                return Ok::<_, io::Error>(None);
            }
            state.principal.ensure_live().map_err(public_io_error)?;
            if !state.entry.active() || Instant::now() >= state.deadline {
                return Err(public_io_error(HostedError::NotFound));
            }
            state
                .storage
                .authorize(
                    &state.principal,
                    &state.entry.object.canonical,
                    McpOperation::Read,
                )
                .map_err(public_io_error)?;
            state
                .storage
                .authorize(
                    &state.principal,
                    &state.entry.object.canonical,
                    McpOperation::PresignDownloadLink,
                )
                .map_err(public_io_error)?;
            let mut buffer = vec![0_u8; state.remaining.min(STREAM_BUFFER_BYTES as u64) as usize];
            let count = tokio::time::timeout_at(
                tokio::time::Instant::from_std(state.deadline.min(state.entry.expires)),
                state.reader.read(&mut buffer),
            )
            .await
            .map_err(|_| public_io_error(HostedError::Unavailable))??;
            state.principal.ensure_live().map_err(public_io_error)?;
            if !state.entry.active() {
                return Err(public_io_error(HostedError::NotFound));
            }
            if count == 0 {
                return Err(public_io_error(HostedError::Unavailable));
            }
            buffer.truncate(count);
            state.remaining -= count as u64;
            state.sent += count as u64;
            if state.remaining == 0 {
                state.finished = true;
                crate::audit::record(
                    Some(&state.principal),
                    "artifact_download",
                    Ok(()),
                    Some(&state.entry.object.canonical),
                    state.sent,
                );
            }
            Ok(Some((Bytes::from(buffer), state)))
        });
        let mut builder = Response::builder()
            .status(if partial {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            })
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"artifact.bin\"",
            )
            .header(header::CACHE_CONTROL, "private, no-store")
            .header("X-Content-Type-Options", "nosniff")
            .header("Referrer-Policy", "no-referrer")
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, (end - start).to_string());
        if partial {
            builder = builder.header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{size}", end - 1),
            );
        }
        builder
            .body(Body::from_stream(stream))
            .map_err(|_| HostedError::Internal)
    }
}

struct DownloadState {
    reader: PinnedReader,
    _stage: StagedFile,
    entry: Arc<GrantEntry>,
    storage: Arc<StoragePlane>,
    principal: Principal,
    remaining: u64,
    sent: u64,
    finished: bool,
    deadline: Instant,
}

impl Drop for DownloadState {
    fn drop(&mut self) {
        if !self.finished {
            crate::audit::record(
                Some(&self.principal),
                "artifact_download",
                if self.remaining == 0 {
                    Ok(())
                } else {
                    Err(HostedError::Unavailable)
                },
                Some(&self.entry.object.canonical),
                self.sent,
            );
        }
    }
}

fn public_io_error(error: HostedError) -> io::Error {
    io::Error::other(error.code())
}

fn parse_range(range: Option<&str>, size: u64) -> Result<(u64, u64, bool)> {
    let Some(range) = range else {
        return Ok((0, size, false));
    };
    let range = range
        .strip_prefix("bytes=")
        .ok_or(HostedError::InvalidRequest)?;
    if range.contains(',') || range.len() > 64 || size == 0 {
        return Err(HostedError::InvalidRequest);
    }
    let (start, end) = range.split_once('-').ok_or(HostedError::InvalidRequest)?;
    if start.is_empty() {
        let suffix = end
            .parse::<u64>()
            .map_err(|_| HostedError::InvalidRequest)?;
        if suffix == 0 {
            return Err(HostedError::InvalidRequest);
        }
        return Ok((size.saturating_sub(suffix), size, true));
    }
    let start = start
        .parse::<u64>()
        .map_err(|_| HostedError::InvalidRequest)?;
    let end = if end.is_empty() {
        size
    } else {
        end.parse::<u64>()
            .map_err(|_| HostedError::InvalidRequest)?
            .saturating_add(1)
            .min(size)
    };
    if start >= size || start >= end {
        return Err(HostedError::InvalidRequest);
    }
    Ok((start, end, true))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, Grant, HostedConfig, Limits, StorageConfig};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use infimount_mcp::{
        McpAccessMode, McpConfirmationRules, McpPathRule, McpRuleSource, StorageRecord,
    };
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};

    async fn delegated_fixture(
        mode: crate::delegation::fixture::Mode,
        timeout: Duration,
    ) -> (
        tempfile::TempDir,
        TransferManager,
        Principal,
        Principal,
        crate::delegation::fixture::MockIssuer,
    ) {
        let (root, storage, original, alice, bob) = fixture();
        drop(original);
        let issuer = crate::delegation::fixture::MockIssuer::start(mode, timeout).await;
        let manager = TransferManager::with_issuer(storage, Some(issuer.issuer.clone()));
        (root, manager, alice, bob, issuer)
    }

    fn fixture_with(
        mut limits: Limits,
        confirm: bool,
    ) -> (
        tempfile::TempDir,
        Arc<StoragePlane>,
        TransferManager,
        Principal,
        Principal,
    ) {
        let root = tempfile::tempdir().unwrap();
        let mut record = StorageRecord::new(
            "artifacts".into(),
            "local".into(),
            json!({"root":root.path()}),
        );
        record.enabled = true;
        record.mcp_exposed = true;
        record.read_only = false;
        record.mcp_policy.default_access = McpAccessMode::None;
        record.mcp_policy.confirmation_rules = McpConfirmationRules {
            require_for_write: confirm,
            require_for_presign: confirm,
            ..Default::default()
        };
        for name in ["alice", "bob"] {
            record.mcp_policy.rules.push(McpPathRule {
                id: name.into(),
                prefix: format!("users/{name}"),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            });
        }
        // This fixture deliberately permits both client IDs at the path-policy
        // level so grant substitution tests exercise owner binding itself.
        let grants = [("alice", "tenant-a"), ("bob", "tenant-b")]
            .into_iter()
            .map(|(subject, tenant)| Grant {
                issuer: "https://issuer.example".into(),
                subject: subject.into(),
                tenant: tenant.into(),
                client_id: None,
                storage_id: record.id.clone(),
                prefix: format!("users/{subject}"),
                read: true,
                create: true,
                download: true,
            })
            .collect();
        limits.json_bytes = 6 * 1024 * 1024;
        let config = Arc::new(HostedConfig {
            public_url: "https://artifacts.example".into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            allow_non_loopback: false,
            auth: AuthConfig {
                issuer: "https://issuer.example".into(),
                audience: "https://artifacts.example/mcp".into(),
                jwks_url: "https://issuer.example/jwks".into(),
                max_token_lifetime_seconds: 3600,
                jwks_cache_seconds: 300,
                jwks_refresh_min_seconds: 30,
            },
            storages: vec![StorageConfig {
                record,
                credentials: BTreeMap::new(),
                dedicated_local: true,
            }],
            grants,
            limits,
            delivery: Default::default(),
            telemetry: Default::default(),
        });
        let storage = Arc::new(StoragePlane::new(config).unwrap());
        let manager = TransferManager::new(storage.clone());
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let principal = |subject: &str, tenant: &str| Principal {
            issuer: "https://issuer.example".into(),
            subject: subject.into(),
            tenant: tenant.into(),
            client_id: "client-a".into(),
            scopes: BTreeSet::from([
                "infimount:read".into(),
                "infimount:create".into(),
                "infimount:download".into(),
            ]),
            expires_at: expires,
        };
        (
            root,
            storage,
            manager,
            principal("alice", "tenant-a"),
            principal("bob", "tenant-b"),
        )
    }

    fn fixture() -> (
        tempfile::TempDir,
        Arc<StoragePlane>,
        TransferManager,
        Principal,
        Principal,
    ) {
        fixture_with(Limits::default(), false)
    }
    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn id(value: &Value, key: &str) -> String {
        value[key].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn binary_upload_read_and_authenticated_download_bind_user_tenant_and_client() {
        let (root, storage, manager, alice, bob) = fixture();
        let bytes = vec![0, 255, 128, 10, 13, 0, 1, 2];
        let path = "/artifacts/users/alice/report.bin";
        let created = manager
            .create_upload(&alice, path, bytes.len() as u64, &sha(&bytes), None, None)
            .await
            .unwrap();
        let upload_id = id(&created, "upload_id");
        assert!(!root.path().join("users/alice/report.bin").exists());
        assert_eq!(
            manager
                .upload(&bob, &upload_id, Body::from(bytes.clone()))
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let chunks = vec![
            Ok::<_, io::Error>(Bytes::copy_from_slice(&bytes[..3])),
            Ok(Bytes::copy_from_slice(&bytes[3..])),
        ];
        manager
            .upload(
                &alice,
                &upload_id,
                Body::from_stream(futures::stream::iter(chunks)),
            )
            .await
            .unwrap();
        assert!(!root.path().join("users/alice/report.bin").exists());
        manager.complete_upload(&alice, &upload_id).await.unwrap();
        assert_eq!(
            manager
                .complete_upload(&alice, &upload_id)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let read = storage
            .read(&alice, path, 0, storage.config().limits.inline_bytes as u64)
            .await
            .unwrap();
        assert_eq!(
            STANDARD.decode(read["content"].as_str().unwrap()).unwrap(),
            bytes
        );
        assert_eq!(
            storage.read(&bob, path, 0, 100).await.unwrap_err(),
            HostedError::NotFound
        );
        let download = manager.create_download(&alice, path, None).await.unwrap();
        assert!(download["url"]
            .as_str()
            .unwrap()
            .starts_with("https://artifacts.example/downloads/"));
        let download_id = id(&download, "download_id");
        assert_eq!(
            manager
                .download(&bob, &download_id, None)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let mut other_client = alice.clone();
        other_client.client_id = "client-b".into();
        assert_eq!(
            manager
                .download(&other_client, &download_id, None)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let response = manager.download(&alice, &download_id, None).await.unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
        assert!(!response.headers().contains_key(header::LOCATION));
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        assert_eq!(
            manager
                .download(&alice, &download_id, None)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        assert_eq!(
            manager
                .write_file(&alice, path, "changed", "utf-8", None)
                .await
                .unwrap_err(),
            HostedError::Conflict
        );
        assert_eq!(
            std::fs::read(root.path().join("users/alice/report.bin")).unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn wrong_checksum_size_and_expired_identity_never_publish_or_keep_reservations() {
        let (root, _, manager, alice, _) = fixture();
        let path = "/artifacts/users/alice/bad.bin";
        let created = manager
            .create_upload(&alice, path, 4, &sha(b"good"), None, None)
            .await
            .unwrap();
        let upload_id = id(&created, "upload_id");
        assert!(manager
            .upload(&alice, &upload_id, Body::from("evil"))
            .await
            .is_err());
        assert_eq!(
            manager
                .complete_upload(&alice, &upload_id)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let created = manager
            .create_upload(&alice, path, 4, &sha(b"good"), None, None)
            .await
            .unwrap();
        assert!(manager
            .upload(
                &alice,
                &id(&created, "upload_id"),
                Body::from("too many bytes")
            )
            .await
            .is_err());
        let mut expired = alice.clone();
        expired.expires_at = 0;
        assert_eq!(
            manager
                .write_file(&expired, path, "good", "utf-8", None)
                .await
                .unwrap_err(),
            HostedError::Unauthorized
        );
        assert!(!root.path().join("users/alice/bad.bin").exists());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let cleared = {
                    let budget = manager.inner.budget.lock().unwrap();
                    budget.bytes == 0 && budget.count == 0 && budget.owners.is_empty()
                };
                if cleared {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled staging worker must release all quota after close");
    }

    #[tokio::test]
    async fn in_flight_upload_has_one_writer_and_cancellation_releases_stage() {
        let (root, _, manager, alice, _) = fixture();
        let created = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/cancel.bin",
                4,
                &sha(b"data"),
                None,
                None,
            )
            .await
            .unwrap();
        let upload_id = id(&created, "upload_id");
        {
            let body = Body::from_stream(futures::stream::pending::<
                std::result::Result<Bytes, io::Error>,
            >());
            let mut pending = Box::pin(manager.upload(&alice, &upload_id, body));
            assert!(futures::poll!(pending.as_mut()).is_pending());
            assert_eq!(
                manager
                    .upload(&alice, &upload_id, Body::from("data"))
                    .await
                    .unwrap_err(),
                HostedError::Conflict
            );
            assert_eq!(
                manager.abort_upload(&alice, &upload_id).await.unwrap_err(),
                HostedError::Conflict
            );
        }
        assert_eq!(
            manager
                .complete_upload(&alice, &upload_id)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);
        assert!(!root.path().join("users/alice/cancel.bin").exists());
        assert_eq!(
            std::fs::read_dir(root.path().join(infimount_core::hosted_io::STAGING_SEGMENT))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn pending_transfers_inline_writes_and_live_response_bodies_share_byte_quota() {
        let limits = Limits {
            inline_bytes: 8,
            transfer_bytes: 8,
            chunk_bytes: 4,
            staging_bytes: 12,
            principal_staging_bytes: 8,
            ..Default::default()
        };
        let (root, _, manager, alice, bob) = fixture_with(limits, false);
        let full = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/full",
                8,
                &sha(b"12345678"),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            manager
                .write_file(&alice, "/artifacts/users/alice/inline", "x", "utf-8", None)
                .await
                .unwrap_err(),
            HostedError::Limit
        );
        let other = manager
            .create_upload(
                &bob,
                "/artifacts/users/bob/four",
                4,
                &sha(b"1234"),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            manager
                .create_upload(
                    &bob,
                    "/artifacts/users/bob/extra",
                    1,
                    &sha(b"x"),
                    None,
                    None
                )
                .await
                .unwrap_err(),
            HostedError::Limit
        );
        manager
            .abort_upload(&alice, &id(&full, "upload_id"))
            .await
            .unwrap();
        manager
            .abort_upload(&bob, &id(&other, "upload_id"))
            .await
            .unwrap();
        std::fs::write(root.path().join("users/alice/source"), b"1234").unwrap();
        let link = manager
            .create_download(&alice, "/artifacts/users/alice/source", None)
            .await
            .unwrap();
        let response = manager
            .download(&alice, &id(&link, "download_id"), None)
            .await
            .unwrap();
        assert_eq!(
            manager
                .create_upload(
                    &alice,
                    "/artifacts/users/alice/eight",
                    8,
                    &sha(b"12345678"),
                    None,
                    None
                )
                .await
                .unwrap_err(),
            HostedError::Limit
        );
        drop(response);
        assert!(manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/eight",
                8,
                &sha(b"12345678"),
                None,
                None
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn confirmation_required_policy_has_no_headless_approval_escape() {
        let (root, storage, manager, alice, _) = fixture_with(Limits::default(), true);
        let path = "/artifacts/users/alice/denied.bin";
        assert_eq!(
            manager
                .write_file(&alice, path, "x", "utf-8", None)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        assert_eq!(
            manager
                .create_upload(&alice, path, 1, &sha(b"x"), None, None)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        std::fs::write(root.path().join("users/alice/readable.bin"), b"x").unwrap();
        assert!(storage
            .read(&alice, "/artifacts/users/alice/readable.bin", 0, 1)
            .await
            .is_ok());
        assert_eq!(
            manager
                .create_download(&alice, "/artifacts/users/alice/readable.bin", None)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        assert!(!root.path().join("users/alice/denied.bin").exists());
    }

    #[tokio::test]
    async fn ambiguous_paths_cursor_substitution_and_missing_scopes_fail_closed() {
        let (root, storage, manager, alice, bob) = fixture();
        for path in [
            "/artifacts/users/alice/../bob/x",
            "/artifacts/users/alice/%2e%2e/bob/x",
            "/artifacts/users/alice\\x",
            "/artifacts/users/alice/.infimount-hosted-staging/x",
        ] {
            assert!(manager
                .write_file(&alice, path, "x", "utf-8", None)
                .await
                .is_err());
        }
        for filename in ["a.bin", "b.bin", "报告.bin"] {
            std::fs::write(root.path().join("users/alice").join(filename), b"x").unwrap();
        }
        let page = storage
            .list(&alice, "/artifacts/users/alice", 1, None)
            .await
            .unwrap();
        let cursor = id(&page, "next_cursor");
        assert_eq!(
            storage
                .list(&bob, "/artifacts/users/bob", 1, Some(cursor))
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let mut no_scopes = alice.clone();
        no_scopes.scopes.clear();
        assert_eq!(
            storage
                .read(&no_scopes, "/artifacts/users/alice/a.bin", 0, 1)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        assert_eq!(
            storage
                .read(&no_scopes, "/unknown-storage/private/file", 0, 1)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        assert_eq!(
            storage
                .stat(&alice, "/artifacts/users/alice/报告.bin")
                .await
                .unwrap()["size_bytes"],
            1
        );
    }

    #[tokio::test]
    async fn link_revalidates_object_identity_and_supports_one_authenticated_range() {
        let (root, _, manager, alice, _) = fixture();
        let path = "/artifacts/users/alice/range.bin";
        std::fs::write(root.path().join("users/alice/range.bin"), b"0123456789").unwrap();
        let link = manager.create_download(&alice, path, None).await.unwrap();
        let response = manager
            .download(&alice, &id(&link, "download_id"), Some("bytes=2-5"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .as_ref(),
            b"2345"
        );
        let link = manager.create_download(&alice, path, None).await.unwrap();
        std::fs::write(root.path().join("users/alice/range.bin"), b"replacement").unwrap();
        assert_eq!(
            manager
                .download(&alice, &id(&link, "download_id"), None)
                .await
                .unwrap_err(),
            HostedError::Conflict
        );
    }

    #[tokio::test]
    async fn idle_expiry_and_restart_invalidate_grants_and_release_private_stages() {
        let (_root, storage, manager, alice, _) = fixture();
        let created = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/expired",
                1,
                &sha(b"x"),
                None,
                Some(1),
            )
            .await
            .unwrap();
        let upload_id = id(&created, "upload_id");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            manager
                .upload(&alice, &upload_id, Body::from("x"))
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);
        let created = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/restart",
                1,
                &sha(b"x"),
                None,
                None,
            )
            .await
            .unwrap();
        let upload_id = id(&created, "upload_id");
        drop(manager);
        let restarted = TransferManager::new(storage);
        assert_eq!(
            restarted
                .complete_upload(&alice, &upload_id)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
    }

    #[test]
    fn proxy_expiry_receipt_reports_lifetime_remaining_after_snapshot_work() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .unwrap()
            .block_on(async {
                let (root, _, manager, alice, _) = fixture();
                let path = "/artifacts/users/alice/delayed-snapshot.bin";
                std::fs::write(root.path().join("users/alice/delayed-snapshot.bin"), b"x").unwrap();

                // Hold the actual filesystem worker pool until the grant's
                // clock has started, so snapshot creation consumes known time.
                // Dropping a sender also releases its worker on assertion failure.
                let mut releases = Vec::new();
                let mut workers = Vec::new();
                for _ in 0..2 {
                    let (release, wait) = std::sync::mpsc::channel();
                    let (started, ready) = tokio::sync::oneshot::channel();
                    workers.push(tokio::task::spawn_blocking(move || {
                        let _ = started.send(());
                        let _ = wait.recv();
                    }));
                    releases.push(release);
                    ready.await.unwrap();
                }
                let mut pending = Box::pin(manager.create_download(&alice, path, Some(30)));
                assert!(futures::poll!(pending.as_mut()).is_pending());
                tokio::time::sleep(Duration::from_millis(1100)).await;
                for release in releases {
                    release.send(()).unwrap();
                }
                for worker in workers {
                    worker.await.unwrap();
                }
                let receipt = tokio::time::timeout(Duration::from_secs(5), pending)
                    .await
                    .unwrap()
                    .unwrap();
                let remaining = receipt["expires_seconds"].as_u64().unwrap();
                assert!(remaining > 0 && remaining < 30);
                assert!(receipt["expires_at_unix_seconds"].as_u64().unwrap() <= alice.expires_at);
            });
    }

    #[tokio::test]
    async fn refreshed_identity_cannot_extend_upload_proxy_or_delegated_epoch_expiry() {
        use crate::delegation::fixture::{MockIssuer, Mode};
        let (root, storage, manager, mut alice, _) = fixture();
        let issuer = MockIssuer::start(Mode::Valid, Duration::from_secs(1)).await;
        let delegated = TransferManager::with_issuer(storage, Some(issuer.issuer.clone()));
        alice.expires_at = crate::now_epoch_seconds() + 5;
        let pending = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/expired-put.bin",
                1,
                &sha(b"x"),
                None,
                Some(30),
            )
            .await
            .unwrap();
        let ready = manager
            .create_upload(
                &alice,
                "/artifacts/users/alice/expired-commit.bin",
                1,
                &sha(b"x"),
                None,
                Some(30),
            )
            .await
            .unwrap();
        manager
            .upload(&alice, &id(&ready, "upload_id"), Body::from("x"))
            .await
            .unwrap();
        let path = "/artifacts/users/alice/expired-download.bin";
        std::fs::write(root.path().join("users/alice/expired-download.bin"), b"x").unwrap();
        let proxy = manager
            .create_download(&alice, path, Some(30))
            .await
            .unwrap();
        let platform = delegated
            .create_download(&alice, path, Some(30))
            .await
            .unwrap();

        // Keep the monotonic deadline live to exercise the absolute authority
        // bound independently. This models the old fractional-second rounding
        // error without depending on the scheduler hitting a subsecond window.
        for grants in [&manager, &delegated] {
            let mut entries = grants.inner.entries.lock().unwrap();
            for entry in entries.values_mut() {
                Arc::get_mut(entry).unwrap().expires = Instant::now() + Duration::from_secs(60);
            }
        }
        let refreshed = Principal {
            expires_at: alice.expires_at + 60,
            ..alice.clone()
        };
        assert_eq!(refreshed.owner_key(), alice.owner_key());
        let remaining = (std::time::UNIX_EPOCH + Duration::from_secs(alice.expires_at))
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        tokio::time::sleep(remaining + Duration::from_millis(25)).await;
        refreshed.ensure_live().unwrap();

        // Exercise every public redemption route before assertions, including
        // the ready-upload path that would otherwise publish after expiration.
        let put = manager
            .upload(&refreshed, &id(&pending, "upload_id"), Body::from("x"))
            .await;
        let complete = manager
            .complete_upload(&refreshed, &id(&ready, "upload_id"))
            .await;
        let proxy_get = manager
            .download(&refreshed, &id(&proxy, "download_id"), None)
            .await;
        let delegated_get = delegated
            .download(&refreshed, &id(&platform, "download_id"), None)
            .await;
        assert_eq!(put.unwrap_err(), HostedError::NotFound);
        assert_eq!(complete.unwrap_err(), HostedError::NotFound);
        assert_eq!(proxy_get.unwrap_err(), HostedError::NotFound);
        assert_eq!(delegated_get.unwrap_err(), HostedError::NotFound);
        assert!(!root.path().join("users/alice/expired-put.bin").exists());
        assert!(!root.path().join("users/alice/expired-commit.bin").exists());
        for receipt in [&pending, &ready, &proxy, &platform] {
            assert_eq!(receipt["expires_at_unix_seconds"], alice.expires_at);
        }
        for grants in [&manager, &delegated] {
            let budget = grants.inner.budget.lock().unwrap();
            assert_eq!(budget.bytes, 0);
            assert_eq!(budget.count, 0);
            assert!(budget.owners.is_empty());
        }
    }

    #[tokio::test]
    async fn unsupported_local_content_type_has_no_file_or_parent_side_effects() {
        let (root, _, manager, alice, _) = fixture();
        let path = "/artifacts/users/alice/new-parent/report.pdf";
        assert_eq!(
            manager
                .write_file(&alice, path, "pdf", "utf-8", Some("application/pdf"))
                .await
                .unwrap_err(),
            HostedError::Unsupported
        );
        assert_eq!(
            manager
                .create_upload(&alice, path, 3, &sha(b"pdf"), Some("application/pdf"), None)
                .await
                .unwrap_err(),
            HostedError::Unsupported
        );
        assert!(!root.path().join("users/alice/new-parent").exists());
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);
    }

    #[tokio::test]
    async fn delegated_platform_url_retains_full_identity_object_and_single_use_redemption() {
        use crate::delegation::fixture::{Mode, ORIGIN, TOKEN};
        let (root, manager, alice, bob, issuer) =
            delegated_fixture(Mode::Valid, Duration::from_secs(1)).await;
        let path = "/artifacts/users/alice/private-report.pdf";
        let bytes = [0, 255, 127, 4, 13, 10];
        std::fs::write(root.path().join("users/alice/private-report.pdf"), bytes).unwrap();
        let link = manager
            .create_download(&alice, path, Some(30))
            .await
            .unwrap();
        let download_id = id(&link, "download_id");
        assert_eq!(link["delivery"], "delegated");
        assert_eq!(link["url"], format!("{ORIGIN}/artifacts/{download_id}"));
        assert_eq!(
            link["redeem_url"],
            format!("https://artifacts.example/downloads/{download_id}")
        );
        assert_eq!(
            link["authorization_audience"],
            "https://artifacts.example/mcp"
        );
        assert_eq!(link["requires_authorization"], true);
        assert_eq!(link["single_use"], true);
        assert!(link["expires_at_unix_seconds"].as_u64().unwrap() <= alice.expires_at);
        let request = issuer.requests.lock().unwrap()[0].clone();
        assert_eq!(request.authorization, format!("Bearer {TOKEN}"));
        assert_eq!(request.body.as_object().unwrap().len(), 6);
        for private in [
            path,
            "private-report.pdf",
            "tenant-a",
            "client-a",
            "alice",
            "sha256",
            "access_token",
        ] {
            assert!(!request.body.to_string().contains(private));
        }
        for stranger in [
            bob,
            Principal {
                client_id: "another-client".into(),
                ..alice.clone()
            },
            Principal {
                tenant: "another-tenant".into(),
                ..alice.clone()
            },
            Principal {
                issuer: "https://another-issuer.example".into(),
                ..alice.clone()
            },
        ] {
            assert_eq!(
                manager
                    .download(&stranger, &download_id, None)
                    .await
                    .unwrap_err(),
                HostedError::NotFound
            );
        }
        let mut missing_scope = alice.clone();
        missing_scope.scopes.remove("infimount:download");
        assert_eq!(
            manager
                .download(&missing_scope, &download_id, None)
                .await
                .unwrap_err(),
            HostedError::Forbidden
        );
        let response = manager.download(&alice, &download_id, None).await.unwrap();
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        assert_eq!(
            manager
                .download(&alice, &download_id, None)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);

        let link = manager
            .create_download(&alice, path, Some(30))
            .await
            .unwrap();
        std::fs::write(
            root.path().join("users/alice/private-report.pdf"),
            b"different object",
        )
        .unwrap();
        assert_eq!(
            manager
                .download(&alice, &id(&link, "download_id"), None)
                .await
                .unwrap_err(),
            HostedError::Conflict
        );
    }

    #[tokio::test]
    async fn issuer_failure_timeout_and_invalid_response_drop_every_provisional_grant() {
        use crate::delegation::fixture::Mode;
        for mode in [
            Mode::Failure,
            Mode::WrongOrigin,
            Mode::WrongGrant,
            Mode::ChangedExpiry,
            Mode::UnknownField,
            Mode::Paused,
        ] {
            let (root, manager, alice, _, issuer) =
                delegated_fixture(mode, Duration::from_millis(100)).await;
            let path = "/artifacts/users/alice/private.bin";
            std::fs::write(
                root.path().join("users/alice/private.bin"),
                b"private bytes",
            )
            .unwrap();
            assert_eq!(
                manager
                    .create_download(&alice, path, Some(30))
                    .await
                    .unwrap_err(),
                HostedError::Unavailable
            );
            assert!(manager.inner.entries.lock().unwrap().is_empty());
            {
                let budget = manager.inner.budget.lock().unwrap();
                assert_eq!(budget.bytes, 0);
                assert_eq!(budget.count, 0);
                assert!(budget.owners.is_empty());
            }
            let grant_id = issuer.requests.lock().unwrap()[0].body["grant_id"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                manager.download(&alice, &grant_id, None).await.unwrap_err(),
                HostedError::NotFound
            );
        }
    }

    #[tokio::test]
    async fn cancelled_issuer_call_never_publishes_a_grant_and_releases_snapshot_quota() {
        use crate::delegation::fixture::Mode;
        let (root, manager, alice, _, issuer) =
            delegated_fixture(Mode::Paused, Duration::from_secs(5)).await;
        let path = "/artifacts/users/alice/cancel-link.bin";
        std::fs::write(
            root.path().join("users/alice/cancel-link.bin"),
            vec![7; 128 * 1024],
        )
        .unwrap();
        let pending_manager = manager.clone();
        let pending_principal = alice.clone();
        let pending = tokio::spawn(async move {
            pending_manager
                .create_download(&pending_principal, path, Some(30))
                .await
        });
        issuer.wait_started().await;
        assert!(manager.inner.entries.lock().unwrap().is_empty());
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 128 * 1024);
        let grant_id = issuer.requests.lock().unwrap()[0].body["grant_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            manager.download(&alice, &grant_id, None).await.unwrap_err(),
            HostedError::NotFound
        );
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert!(manager.inner.entries.lock().unwrap().is_empty());
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);
        assert_eq!(manager.inner.budget.lock().unwrap().count, 0);
        issuer.release.notify_one();
        assert_eq!(
            manager.download(&alice, &grant_id, None).await.unwrap_err(),
            HostedError::NotFound
        );
    }

    #[tokio::test]
    async fn object_change_during_issuance_and_expired_delegated_grants_fail_closed() {
        use crate::delegation::fixture::Mode;
        let (root, manager, alice, _, issuer) =
            delegated_fixture(Mode::Paused, Duration::from_secs(5)).await;
        let path = "/artifacts/users/alice/changed.bin";
        std::fs::write(root.path().join("users/alice/changed.bin"), b"original").unwrap();
        let task_manager = manager.clone();
        let task_principal = alice.clone();
        let pending = tokio::spawn(async move {
            task_manager
                .create_download(&task_principal, path, Some(30))
                .await
        });
        issuer.wait_started().await;
        std::fs::write(root.path().join("users/alice/changed.bin"), b"replacement").unwrap();
        issuer.release.notify_one();
        assert_eq!(pending.await.unwrap().unwrap_err(), HostedError::Conflict);
        assert!(manager.inner.entries.lock().unwrap().is_empty());
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);

        let (root, manager, alice, _, _issuer) =
            delegated_fixture(Mode::Valid, Duration::from_secs(1)).await;
        std::fs::write(root.path().join("users/alice/expiry.bin"), b"expired").unwrap();
        let link = manager
            .create_download(&alice, "/artifacts/users/alice/expiry.bin", Some(1))
            .await
            .unwrap();
        let epoch = link["expires_at_unix_seconds"].as_u64().unwrap();
        let remaining = (std::time::UNIX_EPOCH + Duration::from_secs(epoch))
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        tokio::time::sleep(remaining + Duration::from_millis(25)).await;
        assert_eq!(
            manager
                .download(&alice, &id(&link, "download_id"), None)
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        assert_eq!(manager.inner.budget.lock().unwrap().bytes, 0);
    }

    #[tokio::test]
    async fn delegated_failure_audit_redacts_issuer_payloads_urls_grants_and_paths() {
        use crate::delegation::fixture::{Mode, ORIGIN, TOKEN};
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (root, manager, alice, _, issuer) =
            delegated_fixture(Mode::Failure, Duration::from_secs(1)).await;
        let path = "/artifacts/users/alice/never-log-private-filename.bin";
        std::fs::write(
            root.path()
                .join("users/alice/never-log-private-filename.bin"),
            b"never-log-file-content",
        )
        .unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(captured.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        // This is the only test installing a global subscriber. A temporary
        // future-scoped subscriber races shared callsite registration from
        // parallel tests that emit the same audit callsite without one.
        // Production main also installs a global subscriber once.
        tracing::subscriber::set_global_default(subscriber).unwrap();
        assert_eq!(
            manager
                .create_download(&alice, path, Some(30))
                .await
                .unwrap_err(),
            HostedError::Unavailable
        );
        let text = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(text.contains("delegate_download"));
        assert!(text.contains(&sha(path.as_bytes())));
        let grant = issuer.requests.lock().unwrap()[0].body["grant_id"]
            .as_str()
            .unwrap()
            .to_owned();
        for private in [
            path,
            "never-log-private-filename.bin",
            "never-log-file-content",
            "never-log-issuer-response",
            TOKEN,
            ORIGIN,
            &grant,
        ] {
            assert!(!text.contains(private));
        }
    }

    #[tokio::test]
    async fn sequential_pagination_replaces_cursors_and_wrong_owner_cannot_consume_one() {
        let (root, storage, _manager, alice, bob) = fixture();
        for index in 0..24 {
            std::fs::write(
                root.path().join(format!("users/alice/{index:02}.bin")),
                b"x",
            )
            .unwrap();
        }
        let first = storage
            .list(&alice, "/artifacts/users/alice", 1, None)
            .await
            .unwrap();
        let first_cursor = id(&first, "next_cursor");
        assert_eq!(
            storage
                .list(&bob, "/artifacts/users/bob", 1, Some(first_cursor.clone()))
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
        let mut seen = first["entries"].as_array().unwrap().len();
        let mut cursor = Some(first_cursor.clone());
        while let Some(token) = cursor {
            let page = storage
                .list(&alice, "/artifacts/users/alice", 1, Some(token))
                .await
                .unwrap();
            seen += page["entries"].as_array().unwrap().len();
            cursor = page["next_cursor"].as_str().map(str::to_owned);
        }
        assert_eq!(seen, 24);
        assert_eq!(
            storage
                .list(&alice, "/artifacts/users/alice", 1, Some(first_cursor))
                .await
                .unwrap_err(),
            HostedError::NotFound
        );
    }
}
