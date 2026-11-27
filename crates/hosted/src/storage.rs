//! One authority boundary shared by hosted MCP and artifact transfer routes.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures::io::AsyncReadExt;
use infimount_core::hosted_io::{
    validate_relative_path, ConfinedRoot, FileIdentity, PinnedFile, StagedFile,
};
use infimount_core::{CoreError, Source, SourceKind};
use infimount_mcp::{evaluate_storage_policy, McpOperation, PolicyDecision, StorageRecord};
use opendal::Operator;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Principal;
use crate::config::{Grant, HostedConfig};
use crate::error::{HostedError, Result};

pub struct StoragePlane {
    config: Arc<HostedConfig>,
    storages: HashMap<String, Arc<HostedStorage>>,
    cursors: Mutex<HashMap<String, Cursor>>,
}

pub(crate) struct HostedStorage {
    pub record: StorageRecord,
    backend: Backend,
    cleanup_failed: Arc<AtomicBool>,
}

enum Backend {
    Local(ConfinedRoot),
    Remote(Operator),
}

#[derive(Clone)]
pub(crate) struct AuthorizedObject {
    pub storage: Arc<HostedStorage>,
    pub relative: String,
    pub canonical: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ObjectIdentity {
    pub size: u64,
    version: Option<String>,
    etag: Option<String>,
    local: Option<FileIdentity>,
}

pub(crate) struct PreparedRead {
    pub object: AuthorizedObject,
    pub identity: ObjectIdentity,
    operator: Operator,
    key: String,
    _pin: Option<PinnedFile>,
}

impl PreparedRead {
    pub async fn reader(&self, offset: u64, end: u64) -> Result<opendal::FuturesAsyncReader> {
        let mut request = self.operator.reader_with(&self.key);
        if self.identity.local.is_none() {
            if let Some(version) = &self.identity.version {
                if !self.operator.info().capability().read_with_version {
                    return Err(HostedError::Unsupported);
                }
                request = request.version(version);
            } else if let Some(etag) = &self.identity.etag {
                if !self.operator.info().capability().read_with_if_match {
                    return Err(HostedError::Unsupported);
                }
                request = request.if_match(etag);
            } else {
                return Err(HostedError::Unsupported);
            }
        }
        request
            .await
            .map_err(map_storage)?
            .into_futures_async_read(offset..end)
            .await
            .map_err(map_storage)
    }
}

#[derive(Clone)]
struct Cursor {
    owner: String,
    canonical: String,
    revision: u64,
    expires: Instant,
    backend: Option<String>,
    offset: usize,
}

fn within(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn matches_principal(grant: &Grant, principal: &Principal) -> bool {
    grant.issuer == principal.issuer
        && grant.subject == principal.subject
        && grant.tenant == principal.tenant
        && grant
            .client_id
            .as_ref()
            .is_none_or(|client| client == &principal.client_id)
}

impl StoragePlane {
    pub fn new(config: Arc<HostedConfig>) -> Result<Self> {
        config.validate()?;
        if !cfg!(target_os = "linux") {
            return Err(HostedError::Unsupported);
        }
        let mut storages = HashMap::new();
        for storage in &config.storages {
            let resolved = storage.resolved_record()?;
            let backend = match resolved.backend.as_str() {
                "local" => {
                    let path = resolved
                        .config
                        .get("root")
                        .and_then(Value::as_str)
                        .ok_or(HostedError::Config("local root missing"))?;
                    let root = ConfinedRoot::open(std::path::Path::new(path)).map_err(map_core)?;
                    // These prefixes are operator-provisioned deployment config,
                    // never values supplied by a token or request.
                    for grant in config
                        .grants
                        .iter()
                        .filter(|grant| grant.storage_id == resolved.id && grant.create)
                    {
                        root.ensure_directory(&grant.prefix).map_err(map_core)?;
                    }
                    Backend::Local(root)
                }
                "s3" => {
                    let source = Source {
                        id: resolved.id.clone(),
                        name: resolved.name.clone(),
                        kind: SourceKind::S3,
                        root: String::new(),
                        config: resolved.config.clone(),
                    };
                    let operator = infimount_core::registry::build_hosted_s3_operator(&source)
                        .map_err(map_core)?;
                    if !operator.info().capability().write_with_if_not_exists {
                        return Err(HostedError::Unsupported);
                    }
                    Backend::Remote(operator)
                }
                _ => return Err(HostedError::Unsupported),
            };
            // Do not retain expanded credentials in the public policy record.
            storages.insert(
                storage.record.name.clone(),
                Arc::new(HostedStorage {
                    record: storage.record.clone(),
                    backend,
                    cleanup_failed: Arc::new(AtomicBool::new(false)),
                }),
            );
        }
        Ok(Self {
            config,
            storages,
            cursors: Mutex::new(HashMap::new()),
        })
    }

    pub fn config(&self) -> &HostedConfig {
        &self.config
    }

    pub async fn ready(&self) -> Result<()> {
        for storage in self.storages.values() {
            if storage.cleanup_failed.load(Ordering::Acquire) {
                return Err(HostedError::Unavailable);
            }
            match &storage.backend {
                Backend::Local(root) => {
                    root.stat("").await.map_err(map_core)?;
                }
                Backend::Remote(op) => {
                    op.check().await.map_err(map_storage)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn authorize(
        &self,
        principal: &Principal,
        path: &str,
        operation: McpOperation,
    ) -> Result<AuthorizedObject> {
        principal.ensure_live()?;
        let relative = path.strip_prefix('/').ok_or(HostedError::InvalidRequest)?;
        validate_relative_path(relative, false).map_err(|_| HostedError::InvalidRequest)?;
        let (name, relative) = relative.split_once('/').ok_or(HostedError::NotFound)?;
        let scope = match operation {
            McpOperation::Write | McpOperation::Upload | McpOperation::Mkdir => "infimount:create",
            McpOperation::PresignDownloadLink => "infimount:download",
            McpOperation::Read | McpOperation::Metadata | McpOperation::List => "infimount:read",
            _ => return Err(HostedError::Forbidden),
        };
        if !principal.has_scope(scope) {
            return Err(HostedError::Forbidden);
        }
        let storage = self.storages.get(name).ok_or(HostedError::NotFound)?;
        if !self.config.grants.iter().any(|grant| {
            matches_principal(grant, principal)
                && grant.storage_id == storage.record.id
                && within(relative, &grant.prefix)
                && match operation {
                    McpOperation::Write | McpOperation::Upload | McpOperation::Mkdir => {
                        grant.create
                    }
                    McpOperation::PresignDownloadLink => grant.download && grant.read,
                    _ => grant.read,
                }
        }) {
            return Err(HostedError::NotFound);
        }
        if !storage.record.enabled || !storage.record.mcp_exposed {
            return Err(HostedError::NotFound);
        }
        let evaluation =
            evaluate_storage_policy(&storage.record, relative, operation, false, false)
                .map_err(|_| HostedError::Forbidden)?;
        if !matches!(evaluation.decision, PolicyDecision::Allow) {
            return Err(HostedError::Forbidden);
        }
        if operation.is_write_like() && storage.cleanup_failed.load(Ordering::Acquire) {
            return Err(HostedError::Unavailable);
        }
        Ok(AuthorizedObject {
            storage: storage.clone(),
            relative: relative.into(),
            canonical: path.into(),
        })
    }

    pub async fn list(
        &self,
        principal: &Principal,
        path: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<Value> {
        principal.ensure_live()?;
        if limit == 0 || limit > 200 {
            return Err(HostedError::InvalidRequest);
        }
        if path == "/" {
            if cursor.is_some() {
                return Err(HostedError::InvalidRequest);
            }
            let mut entries = Vec::new();
            for grant in self
                .config
                .grants
                .iter()
                .filter(|grant| matches_principal(grant, principal) && grant.read)
            {
                if let Some(storage) = self
                    .storages
                    .values()
                    .find(|storage| storage.record.id == grant.storage_id)
                {
                    let path = format!("/{}/{}", storage.record.name, grant.prefix);
                    if self.authorize(principal, &path, McpOperation::List).is_ok() {
                        entries.push(json!({"name":grant.prefix.rsplit('/').next(),"path":path,"entry_type":"dir"}));
                    }
                }
            }
            entries.sort_by_key(|entry| entry["path"].as_str().unwrap_or_default().to_owned());
            entries.dedup();
            if entries.len() > limit as usize {
                return Err(HostedError::Limit);
            }
            return Ok(json!({"path":"/","entries":entries,"next_cursor":null,"truncated":false}));
        }
        let object = self.authorize(principal, path, McpOperation::List)?;
        let old = if let Some(cursor) = cursor {
            let mut cursors = self.cursors.lock().map_err(|_| HostedError::Internal)?;
            cursors.retain(|_, cursor| cursor.expires > Instant::now());
            let old = cursors.get(&cursor).ok_or(HostedError::NotFound)?;
            if old.owner != principal.owner_key()
                || old.canonical != path
                || old.revision != object.storage.record.revision
            {
                return Err(HostedError::NotFound);
            }
            // A cursor is an owner-bound, single-use continuation. Claim it
            // under the lock before backend work so replay/concurrent requests
            // cannot create unbounded replacement cursors. A failed page can
            // be restarted from the original list query.
            cursors.remove(&cursor)
        } else {
            None
        };
        let (entries, next_backend, next_offset, truncated) = match &object.storage.backend {
            Backend::Local(root) => {
                let (entries, next) = root
                    .list_page_with_filter(
                        &object.relative,
                        old.as_ref().map_or(0, |cursor| cursor.offset),
                        limit as usize,
                        |candidate| {
                            self.authorize(
                                principal,
                                &format!("/{}/{}", object.storage.record.name, candidate),
                                McpOperation::Metadata,
                            )
                            .is_ok()
                        },
                    )
                    .await
                    .map_err(map_core)?;
                (entries, None, next, false)
            }
            Backend::Remote(op) => {
                let page = infimount_core::operations::list_entries_page_with_filter(
                    op,
                    &object.relative,
                    limit,
                    old.as_ref().and_then(|cursor| cursor.backend.clone()),
                    false,
                    object.storage.record.revision,
                    |candidate| {
                        Ok(self
                            .authorize(
                                principal,
                                &format!(
                                    "/{}/{}",
                                    object.storage.record.name,
                                    candidate.trim_end_matches('/')
                                ),
                                McpOperation::Metadata,
                            )
                            .is_ok())
                    },
                )
                .await
                .map_err(map_core)?;
                (page.entries, page.next_cursor, None, page.truncated)
            }
        };
        let entries = entries.into_iter().filter_map(|entry| {
            let canonical = format!("/{}/{}",object.storage.record.name,entry.path.trim_end_matches('/'));
            self.authorize(principal,&canonical,McpOperation::Metadata).ok()?;
            Some(json!({"name":entry.name,"path":canonical,"entry_type":if entry.is_dir {"dir"} else {"file"},"size_bytes":entry.size,"modified_at":entry.modified_at,"etag":entry.etag}))
        }).collect::<Vec<_>>();
        principal.ensure_live()?;
        let next_cursor = if next_backend.is_some() || next_offset.is_some() {
            let mut cursors = self.cursors.lock().map_err(|_| HostedError::Internal)?;
            cursors.retain(|_, cursor| cursor.expires > Instant::now());
            if cursors.len() >= self.config.limits.max_grants
                || cursors
                    .values()
                    .filter(|cursor| cursor.owner == principal.owner_key())
                    .count()
                    >= self.config.limits.max_grants_per_principal
            {
                return Err(HostedError::Limit);
            }
            let id = Uuid::new_v4().to_string();
            cursors.insert(
                id.clone(),
                Cursor {
                    owner: principal.owner_key(),
                    canonical: path.into(),
                    revision: object.storage.record.revision,
                    expires: Instant::now()
                        + Duration::from_secs(self.config.limits.grant_ttl_seconds),
                    backend: next_backend,
                    offset: next_offset.unwrap_or(0),
                },
            );
            Some(id)
        } else {
            None
        };
        Ok(json!({"path":path,"entries":entries,"next_cursor":next_cursor,"truncated":truncated}))
    }

    pub async fn stat(&self, principal: &Principal, path: &str) -> Result<Value> {
        let object = self.authorize(principal, path, McpOperation::Metadata)?;
        let entry = match &object.storage.backend {
            Backend::Local(root) => root.stat(&object.relative).await.map_err(map_core)?,
            Backend::Remote(op) => infimount_core::operations::stat_entry(op, &object.relative)
                .await
                .map_err(map_core)?,
        };
        Ok(
            json!({"path":path,"entry_type":if entry.is_dir {"dir"} else {"file"},"size_bytes":entry.size,"etag":entry.etag,"modified_at":entry.modified_at}),
        )
    }

    pub(crate) async fn prepare_read(
        &self,
        principal: &Principal,
        path: &str,
        download: bool,
    ) -> Result<PreparedRead> {
        let object = self.authorize(principal, path, McpOperation::Read)?;
        if download {
            self.authorize(principal, path, McpOperation::PresignDownloadLink)?;
        }
        let (operator, key, pin, local) = match &object.storage.backend {
            Backend::Local(root) => {
                let pin = root.pin_file(&object.relative).map_err(map_core)?;
                let identity = pin.identity().map_err(|_| HostedError::NotFound)?;
                (
                    pin.operator().clone(),
                    pin.key().to_owned(),
                    Some(pin),
                    Some(identity),
                )
            }
            Backend::Remote(op) => (op.clone(), object.relative.clone(), None, None),
        };
        let metadata = operator.stat(&key).await.map_err(map_storage)?;
        if metadata.is_dir() {
            return Err(HostedError::InvalidRequest);
        }
        let identity = ObjectIdentity {
            size: metadata.content_length(),
            version: metadata.version().map(str::to_owned),
            etag: metadata.etag().map(str::to_owned),
            local,
        };
        Ok(PreparedRead {
            object,
            identity,
            operator,
            key,
            _pin: pin,
        })
    }

    pub(crate) async fn check_identity(
        &self,
        principal: &Principal,
        object: &AuthorizedObject,
        expected: &ObjectIdentity,
        download: bool,
    ) -> Result<()> {
        let current = self
            .prepare_read(principal, &object.canonical, download)
            .await?;
        if current.object.storage.record.revision != object.storage.record.revision
            || &current.identity != expected
        {
            return Err(HostedError::Conflict);
        }
        Ok(())
    }

    pub async fn read(
        &self,
        principal: &Principal,
        path: &str,
        offset: u64,
        max_bytes: u64,
    ) -> Result<Value> {
        if max_bytes == 0 || max_bytes > self.config.limits.inline_bytes as u64 {
            return Err(HostedError::Limit);
        }
        let source = self.prepare_read(principal, path, false).await?;
        if offset > source.identity.size {
            return Err(HostedError::InvalidRequest);
        }
        let end = source.identity.size.min(offset.saturating_add(max_bytes));
        // A backend can ignore Range. Bound locally before allocating beyond
        // the requested bytes, with one extra byte to reject an oversized body.
        let mut reader = source.reader(offset, end).await?.take(end - offset + 1);
        let mut bytes = Vec::with_capacity((end - offset) as usize);
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| HostedError::Unavailable)?;
        if bytes.len() as u64 != end - offset {
            return Err(HostedError::Conflict);
        }
        self.check_identity(principal, &source.object, &source.identity, false)
            .await?;
        Ok(
            json!({"path":path,"encoding":"base64","content":STANDARD.encode(&bytes),"read_bytes":bytes.len(),"offset":offset,"total_size":source.identity.size,"truncated":end<source.identity.size}),
        )
    }

    pub(crate) fn new_stage(
        &self,
        principal: &Principal,
        path: &str,
        size: u64,
        content_type: Option<&str>,
    ) -> Result<(AuthorizedObject, StagedFile)> {
        if size > self.config.limits.transfer_bytes as u64 {
            return Err(HostedError::Limit);
        }
        let object = self.authorize(principal, path, McpOperation::Write)?;
        Self::check_content_type(&object, content_type)?;
        let stage = match &object.storage.backend {
            Backend::Local(root) => root.new_stage(size).map_err(map_core)?,
            Backend::Remote(_) => StagedFile::temporary(size).map_err(map_core)?,
        };
        Ok((object, stage))
    }

    pub(crate) async fn commit_stage(
        &self,
        principal: &Principal,
        object: &AuthorizedObject,
        stage: &mut StagedFile,
        content_type: Option<&str>,
        valid_until: Option<Instant>,
        retention: Option<Arc<dyn Send + Sync>>,
    ) -> Result<()> {
        if valid_until.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(HostedError::NotFound);
        }
        let authorized = self.authorize(principal, &object.canonical, McpOperation::Write)?;
        Self::check_content_type(&authorized, content_type)?;
        if authorized.storage.record.revision != object.storage.record.revision {
            return Err(HostedError::Conflict);
        }
        match &object.storage.backend {
            Backend::Local(root) => {
                if let Some((parent, _)) = object.relative.rsplit_once('/') {
                    // Parent creation is independently authorized at every
                    // ancestor, and confined root traversal cannot follow links.
                    let mut prefix = String::new();
                    for segment in parent.split('/') {
                        if !prefix.is_empty() {
                            prefix.push('/');
                        }
                        prefix.push_str(segment);
                        if root.stat(&prefix).await.is_err() {
                            self.authorize(
                                principal,
                                &format!("/{}/{}", object.storage.record.name, prefix),
                                McpOperation::Mkdir,
                            )?;
                            root.ensure_directory(&prefix).map_err(map_core)?;
                        }
                    }
                }
                self.authorize(principal, &object.canonical, McpOperation::Write)?;
                if valid_until.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(HostedError::NotFound);
                }
                root.publish(stage, &object.relative).map_err(map_core)?;
            }
            Backend::Remote(op) => infimount_core::hosted_io::publish_stage_checked(
                stage,
                op,
                &object.relative,
                content_type,
                Some(infimount_core::hosted_io::PublishCleanup {
                    retention,
                    failed: object.storage.cleanup_failed.clone(),
                }),
                || {
                    if valid_until.is_some_and(|deadline| Instant::now() >= deadline) {
                        return Err(CoreError::Config("transfer expired".into()));
                    }
                    self.authorize(principal, &object.canonical, McpOperation::Write)
                        .map(|_| ())
                        .map_err(|_| CoreError::Config("authority expired".into()))
                },
            )
            .await
            .map_err(map_core)?,
        }
        Ok(())
    }

    pub async fn write(
        &self,
        principal: &Principal,
        path: &str,
        content: &str,
        encoding: &str,
        content_type: Option<&str>,
    ) -> Result<Value> {
        self.write_with_retention(principal, path, content, encoding, content_type, None)
            .await
    }

    pub(crate) async fn write_with_retention(
        &self,
        principal: &Principal,
        path: &str,
        content: &str,
        encoding: &str,
        content_type: Option<&str>,
        retention: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Value> {
        validate_content_type(content_type)?;
        self.authorize(principal, path, McpOperation::Write)?;
        let cap = self.config.limits.inline_bytes.min(4 * 1024 * 1024);
        let bytes = match encoding {
            "utf-8" => {
                if content.len() > cap {
                    return Err(HostedError::Limit);
                }
                content.as_bytes().to_vec()
            }
            "base64" => {
                if content.len() > cap.div_ceil(3) * 4 {
                    return Err(HostedError::Limit);
                }
                STANDARD
                    .decode(content)
                    .map_err(|_| HostedError::InvalidRequest)?
            }
            _ => return Err(HostedError::InvalidRequest),
        };
        if bytes.len() > cap {
            return Err(HostedError::Limit);
        }
        let (object, mut stage) =
            self.new_stage(principal, path, bytes.len() as u64, content_type)?;
        stage.set_resource_retention(retention.clone());
        stage.write_chunk(&bytes).await.map_err(map_core)?;
        stage.finish(None).await.map_err(map_core)?;
        self.commit_stage(
            principal,
            &object,
            &mut stage,
            content_type,
            None,
            retention,
        )
        .await?;
        Ok(
            json!({"path":path,"written_bytes":bytes.len(),"sha256":stage.sha256(),"content_type":content_type.unwrap_or("application/octet-stream"),"overwrite":false}),
        )
    }

    fn check_content_type(object: &AuthorizedObject, content_type: Option<&str>) -> Result<()> {
        validate_content_type(content_type)?;
        if content_type.is_none() {
            return Ok(());
        }
        match &object.storage.backend {
            Backend::Remote(operator) if operator.info().capability().write_with_content_type => {
                Ok(())
            }
            _ => Err(HostedError::Unsupported),
        }
    }
}

pub(crate) fn validate_content_type(value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > 127
            || !value.is_ascii()
            || value.chars().any(char::is_control)
            || !value.contains('/')
    }) {
        return Err(HostedError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn map_storage(error: opendal::Error) -> HostedError {
    match error.kind() {
        opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch => {
            HostedError::Conflict
        }
        opendal::ErrorKind::NotFound | opendal::ErrorKind::PermissionDenied => {
            HostedError::NotFound
        }
        opendal::ErrorKind::Unsupported => HostedError::Unsupported,
        _ => HostedError::Unavailable,
    }
}
pub(crate) fn map_core(error: CoreError) -> HostedError {
    match error {
        CoreError::Storage(error) => map_storage(error),
        CoreError::Io(error) => match error.kind() {
            std::io::ErrorKind::AlreadyExists => HostedError::Conflict,
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                HostedError::NotFound
            }
            std::io::ErrorKind::Unsupported => HostedError::Unsupported,
            _ => HostedError::Unavailable,
        },
        CoreError::Config(_) => HostedError::InvalidRequest,
        _ => HostedError::Unavailable,
    }
}
