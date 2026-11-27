//! Explicit, bounded configuration for the independent hosted process.
//!
//! No desktop registry, keyring, or ambient provider credential lookup is used.
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use infimount_core::{
    contains_plaintext_secrets, discover_secret_field_names, is_secret_field_name,
};
use infimount_mcp::policy::MCP_POLICY_VERSION;
use infimount_mcp::registry::{StorageRecord, STORAGE_RECORD_SCHEMA_VERSION};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::{HostedError, Result};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_CONFIG_STORAGES: usize = 64;
const MAX_CONFIG_GRANTS: usize = 1024;
const STAGING_SEGMENT: &str = ".infimount-hosted-staging";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedConfig {
    pub public_url: String,
    pub listen: SocketAddr,
    #[serde(default)]
    pub allow_non_loopback: bool,
    pub auth: AuthConfig,
    pub storages: Vec<StorageConfig>,
    pub grants: Vec<Grant>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    #[serde(default = "default_token_lifetime")]
    pub max_token_lifetime_seconds: u64,
    #[serde(default = "default_jwks_cache")]
    pub jwks_cache_seconds: u64,
    #[serde(default = "default_jwks_refresh")]
    pub jwks_refresh_min_seconds: u64,
}

fn default_token_lifetime() -> u64 {
    3600
}
fn default_jwks_cache() -> u64 {
    300
}
fn default_jwks_refresh() -> u64 {
    30
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    #[serde(deserialize_with = "deserialize_storage_record")]
    pub record: StorageRecord,
    #[serde(default)]
    pub credentials: BTreeMap<String, SecretRef>,
    #[serde(default)]
    pub dedicated_local: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretRef {
    Env { name: String },
    File { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub issuer: String,
    pub subject: String,
    pub tenant: String,
    #[serde(default)]
    pub client_id: Option<String>,
    pub storage_id: String,
    pub prefix: String,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub create: bool,
    #[serde(default)]
    pub download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub json_bytes: usize,
    pub inline_bytes: usize,
    pub transfer_bytes: usize,
    pub staging_bytes: usize,
    pub principal_staging_bytes: usize,
    pub chunk_bytes: usize,
    pub global_concurrency: usize,
    pub principal_concurrency: usize,
    pub max_grants: usize,
    pub max_grants_per_principal: usize,
    pub grant_ttl_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            json_bytes: 6 * 1024 * 1024,
            inline_bytes: 4 * 1024 * 1024,
            transfer_bytes: 128 * 1024 * 1024,
            staging_bytes: 512 * 1024 * 1024,
            principal_staging_bytes: 128 * 1024 * 1024,
            chunk_bytes: 1024 * 1024,
            global_concurrency: 16,
            principal_concurrency: 4,
            max_grants: 256,
            max_grants_per_principal: 16,
            grant_ttl_seconds: 300,
            request_timeout_seconds: 60,
        }
    }
}

impl HostedConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, MAX_CONFIG_BYTES)
            .map_err(|_| HostedError::Config("configuration file is unreadable or oversized"))?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|_| HostedError::Config("configuration JSON is invalid"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let public = https_url(&self.public_url)?;
        if self.public_url != public.origin().ascii_serialization() {
            return Err(HostedError::Config(
                "public_url must be a canonical HTTPS origin",
            ));
        }
        if !self.listen.ip().is_loopback() && !self.allow_non_loopback {
            return Err(HostedError::Config(
                "non-loopback listen requires explicit opt-in",
            ));
        }
        self.auth.validate()?;
        if self.auth.audience != format!("{}/mcp", self.public_url) {
            return Err(HostedError::Config(
                "audience must equal public_url plus /mcp",
            ));
        }
        self.limits.validate()?;
        if self.storages.is_empty() || self.storages.len() > MAX_CONFIG_STORAGES {
            return Err(HostedError::Config(
                "storage count is outside the supported range",
            ));
        }
        if self.grants.is_empty() || self.grants.len() > MAX_CONFIG_GRANTS {
            return Err(HostedError::Config(
                "grant count is outside the supported range",
            ));
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for storage in &self.storages {
            storage.validate()?;
            if !ids.insert(storage.record.id.as_str())
                || !names.insert(storage.record.name.as_str())
            {
                return Err(HostedError::Config("storage identities must be unique"));
            }
        }
        let mut grant_keys = HashSet::new();
        for grant in &self.grants {
            grant.validate()?;
            if grant.issuer != self.auth.issuer || !ids.contains(grant.storage_id.as_str()) {
                return Err(HostedError::Config(
                    "grant issuer or storage is not configured",
                ));
            }
            if !grant_keys.insert((
                &grant.issuer,
                &grant.subject,
                &grant.tenant,
                &grant.client_id,
                &grant.storage_id,
                &grant.prefix,
            )) {
                return Err(HostedError::Config("duplicate principal grant"));
            }
        }
        Ok(())
    }
}

impl AuthConfig {
    pub fn validate(&self) -> Result<()> {
        https_url(&self.issuer)?;
        https_url(&self.audience)?;
        https_url(&self.jwks_url)?;
        if !(60..=3600).contains(&self.max_token_lifetime_seconds)
            || !(30..=3600).contains(&self.jwks_cache_seconds)
            || !(5..=300).contains(&self.jwks_refresh_min_seconds)
            || self.jwks_refresh_min_seconds * 2 > self.jwks_cache_seconds
        {
            return Err(HostedError::Config(
                "authentication limits are outside the supported range",
            ));
        }
        Ok(())
    }
}

impl Limits {
    pub fn validate(&self) -> Result<()> {
        if self.inline_bytes == 0
            || self.inline_bytes > 4 * 1024 * 1024
            || self.json_bytes < 1024
            || self.json_bytes > 6 * 1024 * 1024
            || self.inline_bytes.div_ceil(3) * 4 + 1024 > self.json_bytes
            || self.transfer_bytes < self.inline_bytes
            || self.transfer_bytes > 1024 * 1024 * 1024
            || self.principal_staging_bytes < self.transfer_bytes
            || self.staging_bytes < self.principal_staging_bytes
            || self.staging_bytes > 4 * 1024 * 1024 * 1024
            || self.chunk_bytes == 0
            || self.chunk_bytes > 4 * 1024 * 1024
            || self.chunk_bytes > self.transfer_bytes
            || !(1..=64).contains(&self.global_concurrency)
            || self.principal_concurrency == 0
            || self.principal_concurrency > self.global_concurrency
            || !(1..=4096).contains(&self.max_grants)
            || self.max_grants_per_principal == 0
            || self.max_grants_per_principal > self.max_grants
            || !(1..=900).contains(&self.grant_ttl_seconds)
            || !(1..=300).contains(&self.request_timeout_seconds)
        {
            return Err(HostedError::Config(
                "resource limits are outside the supported range",
            ));
        }
        Ok(())
    }
}

impl StorageConfig {
    pub fn validate(&self) -> Result<()> {
        let record = &self.record;
        if !valid_identity(&record.id)
            || !valid_identity(&record.name)
            || record.name.len() > 255
            || matches!(
                record.name.as_str(),
                "." | ".." | ".infimount-hosted-stage" | ".infimount-hosted-staging"
            )
            || record.name.contains(['/', '\\', '%'])
            || record.schema_version != STORAGE_RECORD_SCHEMA_VERSION
            || record.revision == 0
            || record.secret_ref.is_some()
            || !record.secret_fields.is_empty()
            || record.mcp_policy.version != MCP_POLICY_VERSION
            || !record.mcp_policy.allowed_paths.is_empty()
        {
            return Err(HostedError::Config(
                "storage record identity or schema is invalid",
            ));
        }
        let config = record
            .config
            .as_object()
            .ok_or(HostedError::Config("storage config must be an object"))?;
        let secret_names = discover_secret_field_names();
        if contains_plaintext_secrets(&record.config, &secret_names) {
            return Err(HostedError::Config(
                "credentials must use explicit secret references",
            ));
        }
        if config.values().any(|value| {
            value
                .as_str()
                .is_none_or(|value| value.len() > 4096 || value.chars().any(char::is_control))
        }) {
            return Err(HostedError::Config(
                "storage config values must be bounded strings",
            ));
        }
        for rule in &record.mcp_policy.rules {
            if !valid_identity(&rule.id)
                || (!rule.prefix.is_empty() && !canonical_prefix(&rule.prefix))
            {
                return Err(HostedError::Config("policy prefixes must be canonical"));
            }
        }
        for prefix in &record.mcp_policy.denied_paths {
            if !prefix.is_empty() && !canonical_prefix(prefix) {
                return Err(HostedError::Config("policy prefixes must be canonical"));
            }
        }
        match record.backend.as_str() {
            "local" => {
                if !self.dedicated_local
                    || !self.credentials.is_empty()
                    || config.keys().any(|key| key != "root")
                {
                    return Err(HostedError::Config(
                        "local storage requires a dedicated root",
                    ));
                }
                let root =
                    config
                        .get("root")
                        .and_then(Value::as_str)
                        .ok_or(HostedError::Config(
                            "local storage requires an absolute root",
                        ))?;
                if !Path::new(root).is_absolute() || root == "/" || root.contains('\0') {
                    return Err(HostedError::Config(
                        "local storage requires an absolute dedicated root",
                    ));
                }
            }
            "s3" => {
                if self.dedicated_local
                    || config.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "bucket" | "bucketName" | "region" | "endpoint"
                        )
                    })
                    || self.credentials.len() != 2
                    || !self.credentials.contains_key("accessKeyId")
                    || !self.credentials.contains_key("secretAccessKey")
                {
                    return Err(HostedError::Config(
                        "S3 storage requires explicit access key references",
                    ));
                }
                let bucket = config.get("bucket");
                let bucket_name = config.get("bucketName");
                if bucket
                    .or(bucket_name)
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.is_empty() || value.trim() != value)
                    || matches!((bucket, bucket_name), (Some(left), Some(right)) if left != right)
                {
                    return Err(HostedError::Config(
                        "S3 storage requires one unambiguous explicit bucket",
                    ));
                }
                if ["region", "endpoint"].into_iter().any(|field| {
                    config
                        .get(field)
                        .and_then(Value::as_str)
                        .is_none_or(|value| value.is_empty() || value.trim() != value)
                }) {
                    return Err(HostedError::Config(
                        "S3 storage requires an explicit region and endpoint",
                    ));
                }
                if let Some(endpoint) = config.get("endpoint") {
                    let endpoint = endpoint
                        .as_str()
                        .ok_or(HostedError::Config("S3 endpoint is invalid"))?;
                    let url = reqwest::Url::parse(endpoint)
                        .map_err(|_| HostedError::Config("S3 endpoint is invalid"))?;
                    let loopback = url.host_str().is_some_and(|host| {
                        host == "localhost"
                            || host
                                .parse::<std::net::IpAddr>()
                                .is_ok_and(|ip| ip.is_loopback())
                    });
                    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.query().is_some()
                        || url.fragment().is_some()
                    {
                        return Err(HostedError::Config(
                            "S3 endpoint requires HTTPS outside loopback",
                        ));
                    }
                }
            }
            _ => {
                return Err(HostedError::Config(
                    "hosted storage backend is not qualified",
                ))
            }
        }
        for (field, secret) in &self.credentials {
            if !is_secret_field_name(field, &secret_names) || config.contains_key(field) {
                return Err(HostedError::Config(
                    "credential reference field is invalid or duplicated",
                ));
            }
            secret.validate()?;
        }
        Ok(())
    }

    pub fn resolved_record(&self) -> Result<StorageRecord> {
        self.validate()?;
        let mut record = self.record.clone();
        let config = record
            .config
            .as_object_mut()
            .ok_or(HostedError::Config("storage config must be an object"))?;
        for (name, secret) in &self.credentials {
            config.insert(name.clone(), Value::String(secret.resolve()?));
        }
        Ok(record)
    }
}

impl SecretRef {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Env { name }
                if !name.is_empty()
                    && name.len() <= 128
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
            {
                Ok(())
            }
            Self::File { path } if path.is_absolute() => Ok(()),
            _ => Err(HostedError::Config("secret reference is invalid")),
        }
    }

    fn resolve(&self) -> Result<String> {
        self.validate()?;
        let bytes = match self {
            Self::Env { name } => std::env::var(name)
                .map(String::into_bytes)
                .map_err(|_| HostedError::Config("referenced credential is unavailable"))?,
            Self::File { path } => read_bounded(path, MAX_SECRET_BYTES)
                .map_err(|_| HostedError::Config("referenced credential is unavailable"))?,
        };
        if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
            return Err(HostedError::Config(
                "referenced credential is empty or oversized",
            ));
        }
        let value = String::from_utf8(bytes)
            .map_err(|_| HostedError::Config("referenced credential is not UTF-8"))?;
        let value = value.trim_end_matches(['\r', '\n']).to_string();
        if value.is_empty() || value.contains('\0') {
            return Err(HostedError::Config("referenced credential is invalid"));
        }
        Ok(value)
    }
}

impl Grant {
    fn validate(&self) -> Result<()> {
        if !valid_identity(&self.subject)
            || !valid_identity(&self.tenant)
            || self
                .client_id
                .as_deref()
                .is_some_and(|value| !valid_identity(value))
            || !canonical_prefix(&self.prefix)
            || !(self.read || self.create || self.download)
        {
            return Err(HostedError::Config("principal grant is invalid"));
        }
        Ok(())
    }
}

pub(crate) fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(crate) fn canonical_prefix(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2048
        && value.trim() == value
        && !value.contains(['\\', '%'])
        && !value.chars().any(char::is_control)
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 255
                && segment != "."
                && segment != ".."
                && segment != STAGING_SEGMENT
                && segment != ".infimount-hosted-stage"
        })
}

fn https_url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| HostedError::Config("absolute HTTPS URL required"))?;
    if value.len() > 2048
        || value.trim() != value
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(HostedError::Config(
            "absolute HTTPS URL without credentials or query required",
        ));
    }
    Ok(url)
}

fn read_bounded(path: &Path, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("regular file required"));
    }
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::other("file exceeds limit"));
    }
    Ok(bytes)
}

fn reject_unknown(value: &Value, allowed: &[&str]) -> std::result::Result<(), &'static str> {
    let map = value.as_object().ok_or("object required")?;
    if map.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unknown configuration field");
    }
    Ok(())
}

fn check_confirmation_fields(value: &Value) -> std::result::Result<(), &'static str> {
    reject_unknown(
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
}

fn deserialize_storage_record<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<StorageRecord, D::Error> {
    // Desktop's legacy structs accept unknown fields for migration. Hosted
    // startup rejects typos rather than silently weakening a configured rule.
    let value = Value::deserialize(deserializer)?;
    let validate = || -> std::result::Result<(), &'static str> {
        reject_unknown(
            &value,
            &[
                "schema_version",
                "id",
                "name",
                "backend",
                "config",
                "secret_ref",
                "secret_fields",
                "enabled",
                "mcp_exposed",
                "read_only",
                "mcp_policy",
                "revision",
                "created_at",
                "updated_at",
            ],
        )?;
        if let Some(policy) = value.get("mcp_policy") {
            reject_unknown(
                policy,
                &[
                    "version",
                    "default_access",
                    "rules",
                    "denied_paths",
                    "confirmation_rules",
                    "allowed_paths",
                ],
            )?;
            if let Some(confirmations) = policy.get("confirmation_rules") {
                check_confirmation_fields(confirmations)?;
            }
            if let Some(rules) = policy.get("rules").and_then(Value::as_array) {
                for rule in rules {
                    reject_unknown(
                        rule,
                        &["id", "prefix", "access", "source", "confirmation_rules"],
                    )?;
                    if let Some(source) = rule.get("source") {
                        reject_unknown(source, &["kind", "workspace_id"])?;
                    }
                    if let Some(confirmations) = rule
                        .get("confirmation_rules")
                        .filter(|value| !value.is_null())
                    {
                        check_confirmation_fields(confirmations)?;
                    }
                }
            }
        }
        Ok(())
    };
    validate().map_err(serde::de::Error::custom)?;
    serde_json::from_value(value).map_err(|_| serde::de::Error::custom("invalid storage record"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn fixture(root: &Path) -> HostedConfig {
        let mut record =
            StorageRecord::new("Artifacts".into(), "local".into(), json!({"root":root}));
        record.id = "storage-a".into();
        record.mcp_exposed = true;
        HostedConfig {
            public_url: "https://storage.example.test".into(),
            listen: "127.0.0.1:8080".parse().unwrap(),
            allow_non_loopback: false,
            auth: crate::auth::test_support::config(),
            storages: vec![StorageConfig {
                record,
                credentials: BTreeMap::new(),
                dedicated_local: true,
            }],
            grants: vec![Grant {
                issuer: "https://issuer.example.test".into(),
                subject: "alice".into(),
                tenant: "tenant-a".into(),
                client_id: Some("agent".into()),
                storage_id: "storage-a".into(),
                prefix: "tenant-a/alice".into(),
                read: true,
                create: false,
                download: false,
            }],
            limits: Limits::default(),
        }
    }

    #[test]
    fn canonical_configuration_round_trips_without_desktop_state() {
        let dir = TempDir::new().unwrap();
        let config = fixture(dir.path());
        config.validate().unwrap();
        let path = dir.path().join("hosted.json");
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let loaded = HostedConfig::from_file(&path).unwrap();
        assert_eq!(loaded.grants[0].prefix, "tenant-a/alice");
        assert_eq!(
            loaded.storages[0].resolved_record().unwrap().secret_ref,
            None
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn checked_in_deployment_examples_match_the_actual_schema() {
        for example in [
            include_str!("../../../deploy/hosted/example-local.json"),
            include_str!("../../../deploy/hosted/example-s3.json"),
        ] {
            let config: HostedConfig = serde_json::from_str(example).unwrap();
            config.validate().unwrap();
            assert_eq!(config.grants.len(), 2);
            assert_ne!(config.grants[0].tenant, config.grants[1].tenant);
            assert_ne!(config.grants[0].prefix, config.grants[1].prefix);
        }
    }

    #[test]
    fn s3_rejects_root_instead_of_silently_ignoring_a_namespace_boundary() {
        let mut config: HostedConfig =
            serde_json::from_str(include_str!("../../../deploy/hosted/example-s3.json")).unwrap();
        config.validate().unwrap();
        config.storages[0].record.config["root"] = json!("tenant-boundary");
        assert!(config.validate().is_err());
    }

    #[test]
    fn s3_requires_explicit_destination_without_ambient_fallback() {
        let initial: HostedConfig =
            serde_json::from_str(include_str!("../../../deploy/hosted/example-s3.json")).unwrap();
        for field in ["bucket", "region", "endpoint"] {
            let mut config = initial.clone();
            config.storages[0]
                .record
                .config
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(config.validate().is_err());
            for value in ["", " ", " padded "] {
                config.storages[0].record.config[field] = json!(value);
                assert!(config.validate().is_err());
            }
        }
        let mut config = initial;
        config.storages[0].record.config["bucketName"] = json!("conflicting-bucket");
        assert!(config.validate().is_err());
        config.storages[0].record.config["bucketName"] =
            config.storages[0].record.config["bucket"].clone();
        assert!(config.validate().is_ok());
        config.storages[0]
            .record
            .config
            .as_object_mut()
            .unwrap()
            .remove("bucket");
        assert!(config.validate().is_ok());
        config.storages[0].record.config["bucketName"] = json!("");
        assert!(config.validate().is_err());
    }

    #[test]
    fn unknown_fields_fail_at_every_authority_boundary() {
        let dir = TempDir::new().unwrap();
        let initial = serde_json::to_value(fixture(dir.path())).unwrap();
        for path in [
            "",
            "/auth",
            "/limits",
            "/storages/0",
            "/storages/0/record",
            "/storages/0/record/mcp_policy",
            "/storages/0/record/mcp_policy/confirmation_rules",
            "/grants/0",
        ] {
            let mut value = initial.clone();
            value
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("typo_allow_all".into(), json!(true));
            assert!(
                serde_json::from_value::<HostedConfig>(value).is_err(),
                "accepted unknown field at {path}"
            );
        }
        let unknown_secret = json!({"kind":"file", "path":"/etc/credentials", "fallback":"allow"});
        assert!(serde_json::from_value::<SecretRef>(unknown_secret).is_err());
    }

    #[test]
    fn rejects_noncanonical_grants_and_untrusted_storage_records() {
        let dir = TempDir::new().unwrap();
        for prefix in [
            "",
            "/alice",
            "alice/",
            "alice//data",
            "alice/../bob",
            "alice/./data",
            "alice%2fbob",
            "alice\\bob",
            "alice/.infimount-hosted-staging/data",
            "alice/.infimount-hosted-stage/data",
            "alice/\0data",
            " alice",
        ] {
            let mut config = fixture(dir.path());
            config.grants[0].prefix = prefix.into();
            assert!(config.validate().is_err(), "accepted unsafe prefix");
        }
        let mut unicode = fixture(dir.path());
        unicode.grants[0].prefix = "tenant-a/中文/😀".into();
        assert!(unicode.validate().is_ok());
        let mut config = fixture(dir.path());
        config.storages[0].record.secret_ref = Some("storage/desktop-keychain".into());
        assert!(config.validate().is_err());
        config.storages[0].record.secret_ref = None;
        config.storages[0].record.config["password"] = json!("do-not-expose-this");
        let error = config.validate().unwrap_err();
        assert!(!error.to_string().contains("do-not-expose-this"));
        let mut config = fixture(dir.path());
        config.storages[0].dedicated_local = false;
        assert!(config.validate().is_err());
        let mut config = fixture(dir.path());
        config.storages.push(config.storages[0].clone());
        assert!(config.validate().is_err());
        let mut config = fixture(dir.path());
        config.grants[0].storage_id = "unconfigured".into();
        assert!(config.validate().is_err());
        let mut config = fixture(dir.path());
        config.grants.push(config.grants[0].clone());
        assert!(config.validate().is_err());
    }

    #[test]
    fn external_listener_and_resource_audience_require_exact_configuration() {
        let dir = TempDir::new().unwrap();
        let mut config = fixture(dir.path());
        config.listen = "0.0.0.0:8080".parse().unwrap();
        assert!(config.validate().is_err());
        config.allow_non_loopback = true;
        assert!(config.validate().is_ok());
        for public in [
            "http://storage.example.test",
            "https://storage.example.test/",
            "https://storage.example.test/app",
            "https://user:password@storage.example.test",
            "https://storage.example.test?token=secret",
        ] {
            config.public_url = public.into();
            assert!(config.validate().is_err());
        }
        let mut config = fixture(dir.path());
        config.auth.audience = "https://storage.example.test/other".into();
        assert!(config.validate().is_err());
        config.auth = crate::auth::test_support::config();
        config.auth.jwks_url = "http://127.0.0.1/jwks".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn quotas_bound_inline_memory_and_aggregate_staging() {
        let mut limits = Limits::default();
        limits.validate().unwrap();
        limits.inline_bytes += 1;
        assert!(limits.validate().is_err());
        limits = Limits::default();
        limits.json_bytes = limits.inline_bytes;
        assert!(limits.validate().is_err());
        limits = Limits::default();
        limits.principal_staging_bytes = limits.transfer_bytes - 1;
        assert!(limits.validate().is_err());
        limits = Limits::default();
        limits.staging_bytes = limits.principal_staging_bytes - 1;
        assert!(limits.validate().is_err());
        limits = Limits::default();
        limits.principal_concurrency = limits.global_concurrency + 1;
        assert!(limits.validate().is_err());
        limits = Limits::default();
        limits.grant_ttl_seconds = 901;
        assert!(limits.validate().is_err());
    }

    #[test]
    fn secret_files_are_bounded_and_only_resolved_into_operator_config() {
        let dir = TempDir::new().unwrap();
        let access = dir.path().join("access");
        let secret = dir.path().join("secret");
        std::fs::write(&access, "synthetic-access\n").unwrap();
        std::fs::write(&secret, "synthetic-private\n").unwrap();
        let record = StorageRecord::new(
            "S3".into(),
            "s3".into(),
            json!({"bucket":"fixture", "region":"us-east-1", "endpoint":"http://127.0.0.1:9000"}),
        );
        let storage = StorageConfig {
            record,
            dedicated_local: false,
            credentials: BTreeMap::from([
                ("accessKeyId".into(), SecretRef::File { path: access }),
                (
                    "secretAccessKey".into(),
                    SecretRef::File {
                        path: secret.clone(),
                    },
                ),
            ]),
        };
        let resolved = storage.resolved_record().unwrap();
        assert_eq!(resolved.config["accessKeyId"], "synthetic-access");
        assert_eq!(resolved.config["secretAccessKey"], "synthetic-private");
        assert!(storage.record.config.get("secretAccessKey").is_none());
        let serialized = serde_json::to_string(&storage).unwrap();
        assert!(!serialized.contains("synthetic-private"));
        std::fs::write(&secret, vec![b'x'; MAX_SECRET_BYTES + 1]).unwrap();
        assert!(storage.resolved_record().is_err());
        std::fs::remove_file(&secret).unwrap();
        assert!(storage.resolved_record().is_err());
    }

    #[test]
    fn oversize_configuration_fails_without_parsing_or_echoing_input() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("large.json");
        std::fs::write(&path, vec![b'x'; MAX_CONFIG_BYTES + 1]).unwrap();
        assert!(matches!(
            HostedConfig::from_file(&path),
            Err(HostedError::Config(_))
        ));
        std::fs::write(
            &path,
            br#"{"sensitive":"fixture-private-token","listen":oops}"#,
        )
        .unwrap();
        let error = match HostedConfig::from_file(&path) {
            Err(error) => error,
            Ok(_) => panic!("accepted malformed config"),
        };
        assert!(!error.to_string().contains("fixture-private-token"));
    }
}
