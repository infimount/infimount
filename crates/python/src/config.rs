//! Configuration belongs to the trusted embedding application, not tool callers.
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use infimount_core::{Source, SourceKind};
use infimount_mcp::policy::{normalize_storage_policy, McpRuleSource, MCP_POLICY_VERSION};
use infimount_mcp::registry::validate_storage_name;
use infimount_mcp::{McpStoragePolicy, StorageRecord};
use opendal::Operator;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_READ_BYTES: usize = infimount_core::models::MAX_READ_RANGE_BYTES as usize;
pub const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    #[serde(rename = "infimount:read")]
    Read,
    #[serde(rename = "infimount:create")]
    Create,
    #[serde(rename = "infimount:delete")]
    Delete,
    #[serde(rename = "infimount:download")]
    Download,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub storage: StorageConfig,
    #[serde(default)]
    pub scopes: BTreeSet<Scope>,
    #[serde(default)]
    pub policy: McpStoragePolicy,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub read_bytes: usize,
    pub write_bytes: usize,
    pub concurrent_operations: usize,
    pub io_timeout_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            read_bytes: MAX_READ_BYTES,
            write_bytes: MAX_WRITE_BYTES,
            concurrent_operations: 4,
            io_timeout_seconds: 30,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageConfig {
    Local {
        name: String,
        root: PathBuf,
        #[serde(default)]
        read_only: bool,
    },
    S3 {
        name: String,
        bucket: String,
        region: String,
        endpoint: String,
        access_key_id: SecretRef,
        secret_access_key: SecretRef,
        #[serde(default)]
        read_only: bool,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretRef {
    Env { name: String },
    File { path: PathBuf },
}

impl SecretRef {
    fn resolve(&self) -> Result<Zeroizing<String>> {
        let bytes = match self {
            Self::Env { name } => {
                if name.is_empty()
                    || name.len() > 128
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return Err(Error::Configuration);
                }
                Zeroizing::new(
                    std::env::var(name)
                        .map_err(|_| Error::Configuration)?
                        .into_bytes(),
                )
            }
            Self::File { path } => {
                if !path.is_absolute() {
                    return Err(Error::Configuration);
                }
                let file = File::open(path).map_err(|_| Error::Configuration)?;
                if !file.metadata().map_err(|_| Error::Configuration)?.is_file() {
                    return Err(Error::Configuration);
                }
                let mut bytes = Zeroizing::new(Vec::new());
                file.take(MAX_SECRET_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error::Configuration)?;
                bytes
            }
        };
        if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
            return Err(Error::Configuration);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| Error::Configuration)?;
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() || text.chars().any(char::is_control) || text.trim() != text {
            return Err(Error::Configuration);
        }
        Ok(Zeroizing::new(text.to_owned()))
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err(Error::Configuration);
        }
        let value: Value = serde_json::from_str(text).map_err(|_| Error::Configuration)?;
        if let Some(policy) = value.get("policy") {
            validate_policy_fields(policy)?;
        }
        let mut config: Self = serde_json::from_value(value).map_err(|_| Error::Configuration)?;
        let limits = config.limits;
        if !(1..=MAX_READ_BYTES).contains(&limits.read_bytes)
            || !(1..=MAX_WRITE_BYTES).contains(&limits.write_bytes)
            || !(1..=16).contains(&limits.concurrent_operations)
            || !(1..=60).contains(&limits.io_timeout_seconds)
            || config.policy.version != MCP_POLICY_VERSION
            || !config.policy.allowed_paths.is_empty()
            || config.policy.rules.len() > 128
            || config.policy.denied_paths.len() > 128
        {
            return Err(Error::Configuration);
        }
        for rule in &config.policy.rules {
            if rule.id.is_empty()
                || rule.id.len() > 128
                || !matches!(rule.source, McpRuleSource::Manual)
                || !canonical_relative(&rule.prefix, false)
            {
                return Err(Error::Configuration);
            }
        }
        if config
            .policy
            .denied_paths
            .iter()
            .any(|path| !canonical_relative(path, true))
        {
            return Err(Error::Configuration);
        }
        normalize_storage_policy(&mut config.policy).map_err(|_| Error::Configuration)?;
        Ok(config)
    }

    pub fn build(self) -> Result<(StorageRecord, Operator, BTreeSet<Scope>, Limits)> {
        let (name, backend, public_config, resolved_config, read_only) = match self.storage {
            StorageConfig::Local {
                name,
                root,
                read_only,
            } => {
                validate_local_root(&root)?;
                let root = std::fs::canonicalize(root).map_err(|_| Error::Configuration)?;
                let root = root.to_str().ok_or(Error::Configuration)?;
                let config = json!({"root":root});
                (name, "local", config.clone(), config, read_only)
            }
            StorageConfig::S3 {
                name,
                bucket,
                region,
                endpoint,
                access_key_id,
                secret_access_key,
                read_only,
            } => {
                validate_endpoint(&endpoint)?;
                if bucket.is_empty()
                    || bucket.len() > 255
                    || bucket.contains(['/', '\\'])
                    || bucket.chars().any(char::is_control)
                    || region.is_empty()
                    || region.len() > 128
                    || region.chars().any(char::is_control)
                {
                    return Err(Error::Configuration);
                }
                let key = access_key_id.resolve()?;
                let secret = secret_access_key.resolve()?;
                let public = json!({"bucket":bucket,"region":region,"endpoint":endpoint});
                let mut resolved = public.clone();
                resolved["accessKeyId"] = Value::String(key.to_string());
                resolved["secretAccessKey"] = Value::String(secret.to_string());
                (name, "s3", public, resolved, read_only)
            }
        };
        if validate_storage_name(&name).map_err(|_| Error::Configuration)? != name {
            return Err(Error::Configuration);
        }
        let mut record = StorageRecord::new(name, backend.into(), public_config);
        record.mcp_exposed = true;
        record.read_only = read_only;
        record.mcp_policy = self.policy;
        // Core cursors are MAC-protected and revision-bound. Give each immutable
        // client its own epoch so another client's cursor cannot be reused.
        record.revision = u64::from_be_bytes(
            uuid::Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("eight-byte revision"),
        );
        let mut source = Source {
            id: record.id.clone(),
            name: record.name.clone(),
            kind: if backend == "local" {
                SourceKind::Local
            } else {
                SourceKind::S3
            },
            root: String::new(),
            config: resolved_config,
        };
        let built = if backend == "local" {
            infimount_core::registry::build_operator(&source)
        } else {
            infimount_core::registry::build_hosted_s3_operator(&source)
        };
        // The operator owns the explicit credentials it needs. Do not keep a
        // second expanded configuration or expose it through a Python property.
        for field in ["accessKeyId", "secretAccessKey"] {
            if let Some(Value::String(secret)) = source.config.get_mut(field) {
                use zeroize::Zeroize;
                secret.zeroize();
            }
        }
        let op = built.map_err(|_| Error::Configuration)?;
        let op = if backend == "s3" {
            op.layer(
                opendal::layers::TimeoutLayer::new()
                    .with_timeout(std::time::Duration::from_secs(
                        self.limits.io_timeout_seconds,
                    ))
                    .with_io_timeout(std::time::Duration::from_secs(
                        self.limits.io_timeout_seconds,
                    )),
            )
        } else {
            // Cancelling Tokio filesystem futures can abandon a queued OS write.
            // Local calls finish with their permit held instead of pretending
            // that a future timeout cancels operating-system filesystem I/O.
            op
        };
        Ok((record, op, self.scopes, self.limits))
    }
}

pub fn canonical_relative(path: &str, allow_empty: bool) -> bool {
    if path.is_empty() {
        return allow_empty;
    }
    path.len() <= 4096
        && path.trim() == path
        && !path.contains(['\\', '%', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && part != ".infimount-hosted-staging"
        })
}

fn validate_local_root(root: &Path) -> Result<()> {
    if !root.is_absolute() || !root.is_dir() {
        return Err(Error::Configuration);
    }
    for ancestor in root.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| Error::Configuration)?;
        if metadata.file_type().is_symlink() {
            return Err(Error::Configuration);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x0400 != 0 {
                return Err(Error::Configuration);
            }
        }
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let parsed = url::Url::parse(endpoint).map_err(|_| Error::Configuration)?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.host_str().is_none()
        || (parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback))
    {
        return Err(Error::Configuration);
    }
    Ok(())
}

fn only_fields(value: &Value, fields: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or(Error::Configuration)?;
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(Error::Configuration);
    }
    Ok(())
}

fn validate_policy_fields(policy: &Value) -> Result<()> {
    only_fields(
        policy,
        &[
            "version",
            "default_access",
            "rules",
            "denied_paths",
            "confirmation_rules",
        ],
    )?;
    let confirmation = |value: &Value| {
        only_fields(
            value,
            &[
                "require_for_write",
                "require_for_overwrite",
                "require_for_delete",
                "require_for_version_delete",
                "require_for_presign",
                "require_for_cross_storage_copy",
            ],
        )
    };
    if let Some(value) = policy.get("confirmation_rules") {
        confirmation(value)?;
    }
    if let Some(rules) = policy.get("rules") {
        for rule in rules.as_array().ok_or(Error::Configuration)? {
            only_fields(
                rule,
                &["id", "prefix", "access", "source", "confirmation_rules"],
            )?;
            if let Some(source) = rule.get("source") {
                only_fields(source, &["kind"])?;
            }
            if let Some(value) = rule
                .get("confirmation_rules")
                .filter(|value| !value.is_null())
            {
                confirmation(value)?;
            }
        }
    }
    Ok(())
}
