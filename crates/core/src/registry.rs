use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures::TryStreamExt;
use indexmap::IndexMap;
#[cfg(not(windows))]
use opendal::services::Sftp;
use opendal::services::{Azblob, Cos, Fs, Ftp, Gcs, Gdrive, Obs, Onedrive, Oss, Webdav, B2, S3};
use opendal::ErrorKind;
use opendal::Operator;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::config;
use crate::models::{CoreError, Result, Source, SourceKind, StorageBackendCapabilities};

pub fn get_capabilities(op: &Operator) -> StorageBackendCapabilities {
    let info = op.info();
    let full = info.capability();
    StorageBackendCapabilities {
        list_with_versions: full.list_with_versions,
        read_with_version: full.read_with_version,
        delete_with_version: full.delete_with_version,
        presign_read: full.presign_read,
        versioning_disabled: false,
        write_with_user_metadata: full.write_with_user_metadata,
    }
}

pub fn check_versioning_disabled(source: &Source) -> Option<bool> {
    match source.kind {
        SourceKind::S3
        | SourceKind::AzureBlob
        | SourceKind::Gcs
        | SourceKind::Oss
        | SourceKind::Cos
        | SourceKind::Obs => config_bool(&source.config, "versioning").map(|enabled| !enabled),
        _ => None,
    }
}

/// Registry that maps source IDs to OpenDAL operators.
///
/// Operators are built lazily from `Source` configuration and cached.
pub struct OperatorRegistry {
    sources: RwLock<IndexMap<String, Source>>,
    operators: RwLock<HashMap<String, Operator>>,
}

impl OperatorRegistry {
    /// Create a new registry from a list of configured sources.
    pub fn new(sources: Vec<Source>) -> Self {
        let mut map = IndexMap::new();
        for src in sources {
            map.insert(src.id.clone(), src);
        }

        Self {
            sources: RwLock::new(map),
            operators: RwLock::new(HashMap::new()),
        }
    }

    /// Return all known sources.
    pub async fn list_sources(&self) -> Vec<Source> {
        self.sources
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>()
    }

    async fn persist_sources(&self) -> Result<()> {
        let all_sources = self
            .sources
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Replace all sources with the provided list, clearing any
    /// cached operators and persisting the new configuration.
    pub async fn replace_sources(&self, sources: Vec<Source>) -> Result<()> {
        for source in &sources {
            validate_source(source)?;
        }

        {
            let mut srcs = self.sources.write().await;
            srcs.clear();
            for s in sources {
                srcs.insert(s.id.clone(), s);
            }
        }

        // Clear all cached operators – they will be rebuilt lazily.
        {
            let mut ops = self.operators.write().await;
            ops.clear();
        }

        self.persist_sources().await
    }

    /// Add a new source and persist configuration.
    pub async fn add_source(&self, source: Source) -> Result<()> {
        validate_source(&source)?;

        {
            let mut sources = self.sources.write().await;
            let key = source.id.clone();
            sources.shift_insert(0, key, source.clone());
        }

        // Clear any existing operator cached for this source (if any).
        {
            let mut ops = self.operators.write().await;
            ops.remove(&source.id);
        }

        // Persist the updated list.
        self.persist_sources().await
    }

    /// Remove a source by id and persist configuration.
    pub async fn remove_source(&self, source_id: &str) -> Result<()> {
        {
            let mut sources = self.sources.write().await;
            sources.shift_remove(source_id);
        }

        // Remove cached operator reference.
        {
            let mut ops = self.operators.write().await;
            ops.remove(source_id);
        }

        self.persist_sources().await
    }

    /// Update an existing source (or add if missing) and persist.
    pub async fn update_source(&self, source: Source) -> Result<()> {
        validate_source(&source)?;

        {
            let mut sources = self.sources.write().await;
            sources.insert(source.id.clone(), source.clone());
        }

        // When updated, clear the operator cache for that source.
        {
            let mut ops = self.operators.write().await;
            ops.remove(&source.id);
        }

        self.persist_sources().await
    }

    /// Get (or lazily build) an operator for the given source ID.
    pub async fn get_operator(&self, source_id: &str) -> Result<Operator> {
        // Fast path: already built.
        if let Some(op) = self.operators.read().await.get(source_id) {
            return Ok(op.clone());
        }

        // Load source configuration.
        let source = {
            let sources = self.sources.read().await;
            sources
                .get(source_id)
                .cloned()
                .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?
        };

        // Build a new operator for this source.
        let op = build_operator(&source)?;

        // Cache and return.
        let mut ops = self.operators.write().await;
        ops.insert(source_id.to_string(), op.clone());
        Ok(op)
    }

    /// Verify whether a source configuration is reachable and valid.
    pub async fn verify_source(&self, source: &Source) -> Result<()> {
        validate_source(source)?;
        let op = build_operator(source)?;
        // Trigger a lightweight backend call to validate auth/endpoint/root.
        let mut lister = match op.lister("").await {
            Ok(l) => l,
            Err(err) if err.kind() == ErrorKind::NotFound => op.lister("/").await?,
            Err(err) => return Err(err.into()),
        };
        let _ = lister.try_next().await?;
        Ok(())
    }
}

pub fn build_operator(source: &Source) -> Result<Operator> {
    match source.kind {
        SourceKind::Local => build_local_operator(source),
        SourceKind::S3 => build_s3_operator(source),
        SourceKind::WebDav => build_webdav_operator(source),
        SourceKind::AzureBlob => build_azure_blob_operator(source),
        SourceKind::Gcs => build_gcs_operator(source),
        SourceKind::B2 => build_b2_operator(source),
        SourceKind::Oss => build_oss_operator(source),
        SourceKind::Cos => build_cos_operator(source),
        SourceKind::Obs => build_obs_operator(source),
        SourceKind::Sftp => build_sftp_operator(source),
        SourceKind::Ftp => build_ftp_operator(source),
        SourceKind::Gdrive => build_gdrive_operator(source),
        SourceKind::Onedrive => build_onedrive_operator(source),
    }
}

fn validate_source(source: &Source) -> Result<()> {
    if matches!(source.kind, SourceKind::Local) {
        validate_local_root(&local_root(source))?;
    }
    Ok(())
}

fn validate_local_root(root: &str) -> Result<()> {
    let expanded = expand_tilde_home(root);
    let normalized = expanded.trim();

    if normalized.is_empty() {
        return Err(CoreError::Config("directory does not exist".to_string()));
    }

    let path = Path::new(normalized);
    if !path.exists() || !path.is_dir() {
        return Err(CoreError::Config(format!(
            "directory does not exist: {}",
            normalized
        )));
    }

    Ok(())
}

fn local_root(source: &Source) -> String {
    // This must stay in sync with `local_root_from_config`; operator construction
    // and namespace identity resolve the same aliases.
    local_root_from_config(&source.config, &source.root)
}

/// Canonical local root resolution shared by operator construction and
/// storage namespace identity. Returns an owned value so callers cannot
/// accidentally diverge on lifetimes.
pub fn local_root_from_config(config: &Value, fallback: &str) -> String {
    config
        .get("root")
        .and_then(|v| v.as_str())
        .or_else(|| config.get("rootPath").and_then(|v| v.as_str()))
        .or_else(|| config.get("path").and_then(|v| v.as_str()))
        .unwrap_or(fallback)
        .to_string()
}

/// Canonical namespace identity fields resolved from the exact same config
/// aliases and defaults the operator builders use.
///
/// The `source_root` legacy form mirrors `Source.root` handling in the
/// builders: S3 `bucket@region`, Azure `account/container`, plain `bucket`
/// for GCS/B2/OSS/COS/OBS, and the endpoint for WebDAV/SFTP/FTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNamespaceFields {
    pub authority: String,
    pub container: String,
    pub root: String,
}

pub fn resolve_namespace_fields(
    kind: &SourceKind,
    source_root: &str,
    config: &Value,
) -> ResolvedNamespaceFields {
    use SourceKind::*;
    match *kind {
        Local => ResolvedNamespaceFields {
            authority: "local".to_string(),
            container: String::new(),
            root: local_root_from_config(config, source_root),
        },
        S3 => {
            let (legacy_bucket, legacy_region) = split_bucket_region(source_root);
            let bucket = config
                .get("bucket")
                .or_else(|| config.get("bucketName"))
                .and_then(|v| v.as_str())
                .or(legacy_bucket)
                .unwrap_or("")
                .to_string();
            let region = config
                .get("region")
                .and_then(|v| v.as_str())
                .or(legacy_region)
                .unwrap_or("")
                .to_string();
            let endpoint = config
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let authority = if !endpoint.trim().is_empty() {
                normalize_endpoint_authority(&endpoint)
            } else if !region.trim().is_empty() {
                region
            } else {
                "aws".to_string()
            };
            ResolvedNamespaceFields {
                authority,
                container: bucket,
                root: String::new(),
            }
        }
        WebDav => {
            let endpoint = config
                .get("serverUrl")
                .or_else(|| config.get("endpoint"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| (!source_root.trim().is_empty()).then_some(source_root));
            let root = config
                .get("rootPath")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ResolvedNamespaceFields {
                authority: endpoint
                    .map(normalize_endpoint_authority)
                    .unwrap_or_default(),
                container: String::new(),
                root,
            }
        }
        AzureBlob => {
            let mut parts = source_root.split('/');
            let legacy_account = parts.next().filter(|s| !s.is_empty());
            let legacy_container = parts.next().filter(|s| !s.is_empty());
            let container = config
                .get("container")
                .or_else(|| config.get("containerName"))
                .and_then(|v| v.as_str())
                .or(legacy_container)
                .unwrap_or("")
                .to_string();
            let account = config
                .get("accountName")
                .and_then(|v| v.as_str())
                .or(legacy_account)
                .unwrap_or("")
                .to_string();
            let endpoint = config
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let authority = if !endpoint.trim().is_empty() {
                normalize_endpoint_authority(&endpoint)
            } else {
                account
            };
            ResolvedNamespaceFields {
                authority,
                container,
                root: String::new(),
            }
        }
        Gcs => {
            let bucket = config
                .get("bucket")
                .or_else(|| config.get("bucketName"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| (!source_root.trim().is_empty()).then_some(source_root))
                .unwrap_or("")
                .to_string();
            let root = config
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let endpoint = config
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ResolvedNamespaceFields {
                authority: normalize_endpoint_authority(&endpoint),
                container: bucket,
                root,
            }
        }
        B2 => {
            let bucket = config
                .get("bucket")
                .or_else(|| config.get("bucketName"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| (!source_root.trim().is_empty()).then_some(source_root))
                .unwrap_or("")
                .to_string();
            let root = config
                .get("rootPath")
                .or_else(|| config.get("root"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ResolvedNamespaceFields {
                authority: String::new(),
                container: bucket,
                root,
            }
        }
        Oss | Cos | Obs => {
            let bucket = config
                .get("bucket")
                .or_else(|| config.get("bucketName"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| (!source_root.trim().is_empty()).then_some(source_root))
                .unwrap_or("")
                .to_string();
            let root = config
                .get("rootPath")
                .or_else(|| config.get("root"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let endpoint = config
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ResolvedNamespaceFields {
                authority: normalize_endpoint_authority(&endpoint),
                container: bucket,
                root,
            }
        }
        Sftp | Ftp => {
            let endpoint = config
                .get("endpoint")
                .or_else(|| config.get("serverUrl"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| (!source_root.trim().is_empty()).then_some(source_root));
            let root = config
                .get("rootPath")
                .or_else(|| config.get("root"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ResolvedNamespaceFields {
                authority: endpoint
                    .map(normalize_endpoint_authority)
                    .unwrap_or_default(),
                container: String::new(),
                root,
            }
        }
        Gdrive | Onedrive => {
            let root = config
                .get("rootPath")
                .or_else(|| config.get("root"))
                .and_then(|v| v.as_str())
                .unwrap_or(source_root)
                .to_string();
            ResolvedNamespaceFields {
                authority: String::new(),
                container: String::new(),
                root,
            }
        }
    }
}

fn split_bucket_region(root: &str) -> (Option<&str>, Option<&str>) {
    let mut parts = root.split('@');
    let bucket = parts.next().filter(|s| !s.is_empty());
    let region = parts.next().filter(|s| !s.is_empty());
    (bucket, region)
}

/// Normalize an endpoint for identity: lowercase scheme and host, strip userinfo,
/// strip an explicit default port, and drop path/query. Returns the raw input when
/// it cannot be parsed so identity remains deterministic rather than empty.
pub fn normalize_endpoint_authority(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let (scheme, rest) = match trimmed.find("://") {
        Some(index) => (trimmed[..index].to_lowercase(), &trimmed[index + 3..]),
        None => ("https".to_string(), trimmed),
    };
    let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host_port = rest.rsplit('@').next().unwrap_or(rest);
    let (host, port) = match host_port.rfind(':') {
        Some(index) => (
            &host_port[..index],
            host_port[index + 1..].parse::<u16>().ok(),
        ),
        None => (host_port, None),
    };
    let host = host.trim_end_matches('.').to_lowercase();
    if host.is_empty() {
        return trimmed.to_string();
    }
    let default_port = match scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        "ftp" => Some(21),
        "sftp" | "ssh" => Some(22),
        _ => None,
    };
    let port_str = match port {
        Some(value) if Some(value) == default_port => String::new(),
        Some(value) => format!(":{value}"),
        None => String::new(),
    };
    format!("{scheme}://{host}{port_str}")
}

fn build_local_operator(source: &Source) -> Result<Operator> {
    let root = local_root(source);
    let root = root.as_str();

    if root.trim().is_empty() {
        return Err(CoreError::Config(
            "local backend requires a root path".to_string(),
        ));
    }

    let expanded = expand_tilde_home(root);
    let builder = Fs::default().root(&expanded);
    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

#[cfg(not(windows))]
fn build_sftp_operator(source: &Source) -> Result<Operator> {
    let mut builder = Sftp::default();

    if !source.root.is_empty() {
        builder = builder.endpoint(&source.root);
    }
    if let Some(endpoint) = source
        .config
        .get("endpoint")
        .or_else(|| source.config.get("serverUrl"))
        .and_then(|v| v.as_str())
    {
        builder = builder.endpoint(endpoint);
    }
    if let Some(user) = source
        .config
        .get("user")
        .or_else(|| source.config.get("username"))
        .and_then(|v| v.as_str())
    {
        builder = builder.user(user);
    }
    if let Some(key_path) = source
        .config
        .get("privateKeyPath")
        .or_else(|| source.config.get("keyPath"))
        .or_else(|| source.config.get("key"))
        .and_then(|v| v.as_str())
    {
        builder = builder.key(key_path);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(strategy) = source
        .config
        .get("knownHostsStrategy")
        .or_else(|| source.config.get("known_hosts_strategy"))
        .and_then(|v| v.as_str())
    {
        builder = builder.known_hosts_strategy(strategy);
    }
    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

#[cfg(windows)]
fn build_sftp_operator(_source: &Source) -> Result<Operator> {
    Err(CoreError::Config(
        "SFTP is not available in Windows builds because OpenDAL's SFTP backend depends on Unix-only OpenSSH support".to_string(),
    ))
}

fn build_ftp_operator(source: &Source) -> Result<Operator> {
    let mut builder = Ftp::default();

    if !source.root.is_empty() {
        builder = builder.endpoint(&source.root);
    }
    if let Some(endpoint) = source
        .config
        .get("endpoint")
        .or_else(|| source.config.get("serverUrl"))
        .and_then(|v| v.as_str())
    {
        builder = builder.endpoint(endpoint);
    }
    if let Some(user) = source
        .config
        .get("user")
        .or_else(|| source.config.get("username"))
        .and_then(|v| v.as_str())
    {
        builder = builder.user(user);
    }
    if let Some(password) = source.config.get("password").and_then(|v| v.as_str()) {
        builder = builder.password(password);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn expand_tilde_home(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "~" {
        return home_dir().unwrap_or_else(|| trimmed.to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!("{home}/{rest}");
        }
    }

    if let Some(rest) = trimmed.strip_prefix("~\\") {
        if let Some(home) = home_dir() {
            return format!("{home}\\{rest}");
        }
    }

    trimmed.to_string()
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
}

fn build_s3_operator(source: &Source) -> Result<Operator> {
    let mut builder = S3::default();

    // root format: "bucket@region" or just "bucket"
    // Legacy support for root string
    if !source.root.is_empty() {
        let mut parts = source.root.split('@');
        if let Some(bucket) = parts.next() {
            if !bucket.is_empty() {
                builder = builder.bucket(bucket);
            }
        }
        if let Some(region) = parts.next() {
            if !region.is_empty() {
                builder = builder.region(region);
            }
        }
    }

    // Config overrides
    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(region) = source.config.get("region").and_then(|v| v.as_str()) {
        builder = builder.region(region);
    }
    if let Some(ak) = source.config.get("accessKeyId").and_then(|v| v.as_str()) {
        builder = builder.access_key_id(ak);
    }
    if let Some(sk) = source
        .config
        .get("secretAccessKey")
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_access_key(sk);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(default_acl) = source.config.get("defaultAcl").and_then(|v| v.as_str()) {
        if !default_acl.trim().is_empty() {
            builder = builder.default_acl(default_acl);
        }
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_webdav_operator(source: &Source) -> Result<Operator> {
    let mut builder = Webdav::default();

    if !source.root.is_empty() {
        builder = builder.endpoint(&source.root);
    }

    if let Some(endpoint) = source
        .config
        .get("serverUrl")
        .or_else(|| source.config.get("endpoint"))
        .and_then(|v| v.as_str())
    {
        builder = builder.endpoint(endpoint);
    }
    if let Some(username) = source.config.get("username").and_then(|v| v.as_str()) {
        builder = builder.username(username);
    }
    if let Some(password) = source.config.get("password").and_then(|v| v.as_str()) {
        builder = builder.password(password);
    }
    if let Some(root) = source.config.get("rootPath").and_then(|v| v.as_str()) {
        builder = builder.root(root);
    }
    if config_bool(&source.config, "disableCreateDir").unwrap_or(false) {
        builder = builder.disable_create_dir(true);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_azure_blob_operator(source: &Source) -> Result<Operator> {
    let mut builder = Azblob::default();

    // root format: "account/container"
    if !source.root.is_empty() {
        let mut parts = source.root.split('/');
        if let Some(account) = parts.next() {
            if !account.is_empty() {
                builder = builder.account_name(account);
            }
        }
        if let Some(container) = parts.next() {
            if !container.is_empty() {
                builder = builder.container(container);
            }
        }
    }

    if let Some(container) = source
        .config
        .get("container")
        .or_else(|| source.config.get("containerName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.container(container);
    }
    if let Some(account_name) = source.config.get("accountName").and_then(|v| v.as_str()) {
        builder = builder.account_name(account_name);
    }
    if let Some(account_key) = source.config.get("accountKey").and_then(|v| v.as_str()) {
        builder = builder.account_key(account_key);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_gdrive_operator(source: &Source) -> Result<Operator> {
    let mut builder = Gdrive::default();

    if !source.root.is_empty() {
        builder = builder.root(&source.root);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(access_token) = source
        .config
        .get("accessToken")
        .or_else(|| source.config.get("access_token"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_token(access_token);
    }
    if let Some(refresh_token) = source
        .config
        .get("refreshToken")
        .or_else(|| source.config.get("refresh_token"))
        .and_then(|v| v.as_str())
    {
        builder = builder.refresh_token(refresh_token);
    }
    if let Some(client_id) = source
        .config
        .get("clientId")
        .or_else(|| source.config.get("client_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.client_id(client_id);
    }
    if let Some(client_secret) = source
        .config
        .get("clientSecret")
        .or_else(|| source.config.get("client_secret"))
        .and_then(|v| v.as_str())
    {
        builder = builder.client_secret(client_secret);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_onedrive_operator(source: &Source) -> Result<Operator> {
    let mut builder = Onedrive::default();

    if !source.root.is_empty() {
        builder = builder.root(&source.root);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(access_token) = source
        .config
        .get("accessToken")
        .or_else(|| source.config.get("access_token"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_token(access_token);
    }
    if let Some(refresh_token) = source
        .config
        .get("refreshToken")
        .or_else(|| source.config.get("refresh_token"))
        .and_then(|v| v.as_str())
    {
        builder = builder.refresh_token(refresh_token);
    }
    if let Some(client_id) = source
        .config
        .get("clientId")
        .or_else(|| source.config.get("client_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.client_id(client_id);
    }
    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_gcs_operator(source: &Source) -> Result<Operator> {
    let mut builder = Gcs::default();

    // root format: "bucket"
    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(root) = source.config.get("root").and_then(|v| v.as_str()) {
        builder = builder.root(root);
    }

    let credential = source
        .config
        .get("credential")
        .and_then(|v| v.as_str())
        .or_else(|| {
            source
                .config
                .get("serviceAccountJson")
                .and_then(|v| v.as_str())
        })
        .and_then(normalize_gcs_credential);

    if let Some(encoded) = &credential {
        builder = builder.credential(encoded);
    }

    let credential_path = source
        .config
        .get("credentialPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if let Some(cp) = credential_path {
        builder = builder.credential_path(cp);
    }

    // If endpoint is set (emulator) and no credentials provided,
    // treat this as an anonymous/emulator connection:
    // - don't try to load credentials from env or VM metadata
    // - allow unsigned requests.
    if source.config.get("endpoint").is_some() && credential.is_none() && credential_path.is_none()
    {
        builder = builder
            .skip_signature()
            .disable_vm_metadata()
            .disable_config_load();
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_b2_operator(source: &Source) -> Result<Operator> {
    let mut builder = B2::default();

    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(bucket_id) = source.config.get("bucketId").and_then(|v| v.as_str()) {
        builder = builder.bucket_id(bucket_id);
    }
    if let Some(application_key_id) = source
        .config
        .get("applicationKeyId")
        .or_else(|| source.config.get("keyId"))
        .or_else(|| source.config.get("application_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.application_key_id(application_key_id);
    }
    if let Some(application_key) = source
        .config
        .get("applicationKey")
        .or_else(|| source.config.get("application_key"))
        .and_then(|v| v.as_str())
    {
        builder = builder.application_key(application_key);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_oss_operator(source: &Source) -> Result<Operator> {
    let mut builder = Oss::default();

    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(access_key_id) = source
        .config
        .get("accessKeyId")
        .or_else(|| source.config.get("access_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_id(access_key_id);
    }
    if let Some(access_key_secret) = source
        .config
        .get("accessKeySecret")
        .or_else(|| source.config.get("access_key_secret"))
        .or_else(|| source.config.get("secretAccessKey"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_secret(access_key_secret);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(addressing_style) = source
        .config
        .get("addressingStyle")
        .and_then(|v| v.as_str())
    {
        builder = builder.addressing_style(addressing_style);
    }
    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_cos_operator(source: &Source) -> Result<Operator> {
    let mut builder = Cos::default();

    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(secret_id) = source
        .config
        .get("secretId")
        .or_else(|| source.config.get("secret_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_id(secret_id);
    }
    if let Some(secret_key) = source
        .config
        .get("secretKey")
        .or_else(|| source.config.get("secret_key"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_key(secret_key);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn build_obs_operator(source: &Source) -> Result<Operator> {
    let mut builder = Obs::default();

    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(bucket) = source
        .config
        .get("bucket")
        .or_else(|| source.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = source.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(access_key_id) = source
        .config
        .get("accessKeyId")
        .or_else(|| source.config.get("access_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_id(access_key_id);
    }
    if let Some(secret_access_key) = source
        .config
        .get("secret_access_key")
        .or_else(|| source.config.get("access_key_secret"))
        .or_else(|| source.config.get("secretAccessKey"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_access_key(secret_access_key);
    }
    if let Some(root) = source
        .config
        .get("rootPath")
        .or_else(|| source.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?;
    Ok(op)
}

fn config_bool(config: &Value, key: &str) -> Option<bool> {
    match config.get(key)? {
        Value::Bool(v) => Some(*v),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Some(true),
            "false" | "0" | "no" | "n" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn normalize_gcs_credential(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Some paste sources include a UTF-8 BOM or wrap JSON in quotes. Handle both.
    let trimmed = trimmed.strip_prefix('\u{feff}').unwrap_or(trimmed).trim();
    let trimmed = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len().saturating_sub(1)].trim()
    } else {
        trimmed
    };

    if trimmed.starts_with('{') {
        // OpenDAL/reqsign expects base64-encoded credential content.
        Some(BASE64_STANDARD.encode(trimmed.as_bytes()))
    } else {
        // Treat as base64; strip whitespace/newlines so pasted values still work.
        let cleaned: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Source;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    fn test_config_path() -> PathBuf {
        let mut p = env::temp_dir();
        p.push("infimount_test_config.json");
        p
    }

    fn reset_config_file(path: &PathBuf) {
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn add_remove_source_persists() {
        let cfg = test_config_path();
        reset_config_file(&cfg);
        env::set_var("INFIMOUNT_CONFIG", &cfg);

        let registry = OperatorRegistry::new(vec![]);

        let s = Source {
            id: "test1".to_string(),
            name: "Test1".to_string(),
            kind: crate::models::SourceKind::Local,
            root: "/tmp".to_string(),
            config: serde_json::json!({}),
        };

        registry.add_source(s.clone()).await.unwrap();
        let sources = registry.list_sources().await;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "test1");

        registry.remove_source("test1").await.unwrap();
        let sources = registry.list_sources().await;
        assert_eq!(sources.len(), 0);

        // cleanup
        let _ = fs::remove_file(cfg);
    }

    #[tokio::test]
    async fn sources_are_listed_newest_first() {
        let cfg = test_config_path();
        reset_config_file(&cfg);
        env::set_var("INFIMOUNT_CONFIG", &cfg);

        let registry = OperatorRegistry::new(vec![]);

        let mk = |id: &str| Source {
            id: id.to_string(),
            name: id.to_string(),
            kind: crate::models::SourceKind::Local,
            root: "/tmp".to_string(),
            config: serde_json::json!({}),
        };

        registry.add_source(mk("a")).await.unwrap();
        registry.add_source(mk("b")).await.unwrap();
        registry.add_source(mk("c")).await.unwrap();

        let ids = registry
            .list_sources()
            .await
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["c", "b", "a"]);

        let mut updated_b = mk("b");
        updated_b.name = "updated".to_string();
        registry.update_source(updated_b).await.unwrap();
        let ids_after_update = registry
            .list_sources()
            .await
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert_eq!(ids_after_update, vec!["c", "b", "a"]);

        let _ = fs::remove_file(cfg);
    }

    #[test]
    fn normalize_gcs_credential_encodes_raw_json() {
        let raw = r#"{ "type": "service_account", "project_id": "demo" }"#;
        let encoded = normalize_gcs_credential(raw).expect("credential must exist");
        let decoded = BASE64_STANDARD
            .decode(&encoded)
            .expect("must be valid base64");
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), raw);
    }

    #[test]
    fn normalize_gcs_credential_strips_quotes_and_bom() {
        let raw = "\u{feff}\"{\n  \"type\": \"service_account\"\n}\"";
        let encoded = normalize_gcs_credential(raw).expect("credential must exist");
        let decoded = BASE64_STANDARD
            .decode(&encoded)
            .expect("must be valid base64");
        let decoded_str = std::str::from_utf8(&decoded).unwrap();
        assert!(decoded_str.starts_with('{'));
        assert!(decoded_str.contains("\"type\""));
    }

    #[test]
    fn normalize_gcs_credential_cleans_base64_whitespace() {
        let encoded = BASE64_STANDARD.encode(b"hello");
        let with_ws = format!("  {} \n", encoded);
        assert_eq!(normalize_gcs_credential(&with_ws).unwrap(), encoded);
    }

    #[test]
    fn build_b2_operator_accepts_native_b2_config() {
        let source = Source {
            id: "b2".to_string(),
            name: "B2".to_string(),
            kind: crate::models::SourceKind::B2,
            root: String::new(),
            config: serde_json::json!({
                "bucket": "bucket-name",
                "bucketId": "bucket-id",
                "applicationKeyId": "key-id",
                "applicationKey": "application-key",
                "rootPath": "/workspace"
            }),
        };

        let op = build_operator(&source).expect("operator should build");
        let caps = op.info().capability();
        assert!(caps.list);
        assert!(caps.read);
        assert!(caps.write);
        assert!(caps.presign_read);
        assert!(caps.write_with_user_metadata);
    }

    #[test]
    fn builds_v0_7_object_store_operators() {
        for (kind, config) in [
            (
                crate::models::SourceKind::Oss,
                serde_json::json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://oss-cn-beijing.aliyuncs.com",
                    "accessKeyId": "key-id",
                    "accessKeySecret": "key-secret",
                    "rootPath": "/workspace"
                }),
            ),
            (
                crate::models::SourceKind::Cos,
                serde_json::json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://cos.ap-singapore.myqcloud.com",
                    "secretId": "secret-id",
                    "secretKey": "secret-key",
                    "rootPath": "/workspace"
                }),
            ),
            (
                crate::models::SourceKind::Obs,
                serde_json::json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://obs.cn-north-4.myhuaweicloud.com",
                    "accessKeyId": "key-id",
                    "secret_access_key": "key-secret",
                    "rootPath": "/workspace"
                }),
            ),
        ] {
            let source = Source {
                id: kind.to_string(),
                name: kind.to_string(),
                kind,
                root: String::new(),
                config,
            };
            let op = build_operator(&source).expect("operator should build");
            let caps = op.info().capability();
            assert!(caps.list);
            assert!(caps.read);
            assert!(caps.write);
            assert!(caps.copy);
            assert!(caps.presign_read);
            assert!(!caps.rename);
        }
    }

    #[test]
    fn builds_oauth_drive_operators() {
        for (kind, config, versions) in [
            (
                crate::models::SourceKind::Gdrive,
                serde_json::json!({
                    "refreshToken": "refresh-token",
                    "clientId": "client-id",
                    "clientSecret": "client-secret",
                    "rootPath": "/workspace"
                }),
                false,
            ),
            (
                crate::models::SourceKind::Onedrive,
                serde_json::json!({
                    "refreshToken": "refresh-token",
                    "clientId": "client-id",
                    "rootPath": "/workspace",
                    "versioning": true
                }),
                true,
            ),
        ] {
            let source = Source {
                id: kind.to_string(),
                name: kind.to_string(),
                kind,
                root: String::new(),
                config,
            };
            let op = build_operator(&source).expect("operator should build");
            let caps = op.info().capability();
            assert!(caps.list);
            assert!(caps.read);
            assert!(caps.write);
            assert!(caps.copy);
            assert!(caps.rename);
            assert!(!caps.presign_read);
            assert_eq!(caps.list_with_versions, versions);
        }
    }

    #[test]
    fn builds_ftp_operator() {
        let source = Source {
            id: "ftp".to_string(),
            name: "ftp".to_string(),
            kind: crate::models::SourceKind::Ftp,
            root: String::new(),
            config: serde_json::json!({
                "endpoint": "ftp://example.com:21",
                "user": "alice",
                "password": "password",
                "rootPath": "/workspace",
            }),
        };

        let op = build_operator(&source).expect("operator should build");
        let caps = op.info().capability();
        assert!(caps.list);
        assert!(caps.read);
        assert!(caps.write);
        assert!(!caps.copy);
        assert!(!caps.presign_read);
    }

    #[cfg(not(windows))]
    #[test]
    fn builds_sftp_operator() {
        let source = Source {
            id: "sftp".to_string(),
            name: "sftp".to_string(),
            kind: crate::models::SourceKind::Sftp,
            root: String::new(),
            config: serde_json::json!({
                "endpoint": "ssh://example.com:22",
                "user": "alice",
                "privateKeyPath": "/home/alice/.ssh/id_ed25519",
                "rootPath": "/workspace",
                "knownHostsStrategy": "Strict",
                "enableCopy": true,
            }),
        };

        let op = build_operator(&source).expect("operator should build");
        let caps = op.info().capability();
        assert!(caps.list);
        assert!(caps.read);
        assert!(caps.write);
        assert!(caps.copy);
        assert!(!caps.presign_read);
    }

    #[test]
    fn config_bool_accepts_storage_config_values() {
        assert_eq!(config_bool(&serde_json::json!(true), "key"), None); // it checks nested key
        assert_eq!(
            config_bool(&serde_json::json!({"key": true}), "key"),
            Some(true)
        );
        assert_eq!(
            config_bool(&serde_json::json!({"key": "on"}), "key"),
            Some(true)
        );
        assert_eq!(
            config_bool(&serde_json::json!({"key": "false"}), "key"),
            Some(false)
        );
        assert_eq!(
            config_bool(&serde_json::json!({"key": "off"}), "key"),
            Some(false)
        );
        assert_eq!(
            config_bool(&serde_json::json!({"key": "maybe"}), "key"),
            None
        );
    }

    #[test]
    fn local_validation_accepts_config_root() {
        let source = Source {
            id: "local-config-root".to_string(),
            name: "Local Config Root".to_string(),
            kind: crate::models::SourceKind::Local,
            root: String::new(),
            config: serde_json::json!({ "root": std::env::temp_dir().to_string_lossy() }),
        };

        validate_source(&source).expect("local source should use config.root");
    }

    #[test]
    fn expand_tilde_home_expands_simple_prefix() {
        std::env::set_var("HOME", "/home/testuser");
        assert_eq!(expand_tilde_home("~/Downloads"), "/home/testuser/Downloads");
        assert_eq!(expand_tilde_home("~"), "/home/testuser");
    }

    #[tokio::test]
    async fn add_source_rejects_missing_local_directory() {
        let cfg = test_config_path();
        reset_config_file(&cfg);
        env::set_var("INFIMOUNT_CONFIG", &cfg);

        let registry = OperatorRegistry::new(vec![]);

        let s = Source {
            id: "missing-dir".to_string(),
            name: "Missing Dir".to_string(),
            kind: crate::models::SourceKind::Local,
            root: "/tmp/infimount-this-path-does-not-exist".to_string(),
            config: serde_json::json!({}),
        };

        let err = registry.add_source(s).await.unwrap_err();
        assert!(err.to_string().contains("directory does not exist"));
    }
}
