use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures::TryStreamExt;
use opendal::services::{Azblob, Fs, Gcs, Webdav, S3};
use opendal::ErrorKind;
use opendal::Operator;
use tokio::sync::RwLock;

use crate::config;
use crate::models::{CoreError, Result, Source, SourceKind};

/// Registry that maps source IDs to OpenDAL operators.
///
/// Operators are built lazily from `Source` configuration and cached.
pub struct OperatorRegistry {
    sources: RwLock<HashMap<String, Source>>,
    operators: RwLock<HashMap<String, Operator>>,
}

impl OperatorRegistry {
    /// Create a new registry from a list of configured sources.
    pub fn new(sources: Vec<Source>) -> Self {
        let mut map = HashMap::new();
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

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Add a new source and persist configuration.
    pub async fn add_source(&self, source: Source) -> Result<()> {
        validate_source(&source)?;

        {
            let mut sources = self.sources.write().await;
            sources.insert(source.id.clone(), source.clone());
        }

        // Clear any existing operator cached for this source (if any).
        {
            let mut ops = self.operators.write().await;
            ops.remove(&source.id);
        }

        // Persist the updated list.
        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Remove a source by id and persist configuration.
    pub async fn remove_source(&self, source_id: &str) -> Result<()> {
        {
            let mut sources = self.sources.write().await;
            sources.remove(source_id);
        }

        // Remove cached operator reference.
        {
            let mut ops = self.operators.write().await;
            ops.remove(source_id);
        }

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
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

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
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

fn build_operator(source: &Source) -> Result<Operator> {
    match source.kind {
        SourceKind::Local => build_local_operator(&source.root),
        SourceKind::S3 => build_s3_operator(source),
        SourceKind::WebDav => build_webdav_operator(source),
        SourceKind::AzureBlob => build_azure_blob_operator(source),
        SourceKind::Gcs => build_gcs_operator(source),
    }
}

fn validate_source(source: &Source) -> Result<()> {
    if matches!(source.kind, SourceKind::Local) {
        validate_local_root(&source.root)?;
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

fn build_local_operator(root: &str) -> Result<Operator> {
    let expanded = expand_tilde_home(root);
    let builder = Fs::default().root(&expanded);
    let op = Operator::new(builder).map_err(CoreError::Storage)?.finish();
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

    // Config overrides
    if let Some(config) = &source.config {
        if let Some(bucket) = config.get("bucketName") {
            builder = builder.bucket(bucket);
        }
        if let Some(region) = config.get("region") {
            builder = builder.region(region);
        }
        if let Some(access_key_id) = config.get("accessKeyId") {
            builder = builder.access_key_id(access_key_id);
        }
        if let Some(secret_access_key) = config.get("secretAccessKey") {
            builder = builder.secret_access_key(secret_access_key);
        }
        if let Some(endpoint) = config.get("endpoint") {
            builder = builder.endpoint(endpoint);
        }
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?.finish();
    Ok(op)
}

fn build_webdav_operator(source: &Source) -> Result<Operator> {
    let mut builder = Webdav::default();

    if !source.root.is_empty() {
        builder = builder.endpoint(&source.root);
    }

    if let Some(config) = &source.config {
        if let Some(server_url) = config.get("serverUrl") {
            builder = builder.endpoint(server_url);
        }
        if let Some(username) = config.get("username") {
            builder = builder.username(username);
        }
        if let Some(password) = config.get("password") {
            builder = builder.password(password);
        }
        if let Some(root_path) = config.get("rootPath") {
            builder = builder.root(root_path);
        }
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?.finish();
    Ok(op)
}

fn build_azure_blob_operator(source: &Source) -> Result<Operator> {
    let mut builder = Azblob::default();

    // root format: "account/container"
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

    if let Some(config) = &source.config {
        if let Some(account_name) = config.get("accountName") {
            builder = builder.account_name(account_name);
        }
        if let Some(container_name) = config.get("containerName") {
            builder = builder.container(container_name);
        }
        if let Some(account_key) = config.get("accountKey") {
            builder = builder.account_key(account_key);
        }
        if let Some(endpoint) = config.get("endpoint") {
            builder = builder.endpoint(endpoint);
        }
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?.finish();
    Ok(op)
}

fn build_gcs_operator(source: &Source) -> Result<Operator> {
    let mut builder = Gcs::default();

    // root format: "bucket"
    if !source.root.is_empty() {
        builder = builder.bucket(&source.root);
    }

    if let Some(config) = &source.config {
        if let Some(bucket) = config.get("bucket") {
            builder = builder.bucket(bucket);
        }

        let credential = config
            .get("credential")
            .and_then(|v| normalize_gcs_credential(v));
        if let Some(encoded) = &credential {
            builder = builder.credential(encoded);
        }

        let credential_path = config.get("credentialPath").filter(|s| !s.is_empty());
        if let Some(cp) = credential_path {
            builder = builder.credential_path(cp);
        }

        if let Some(endpoint) = config.get("endpoint") {
            builder = builder.endpoint(endpoint);
        }

        // If endpoint is set (emulator) and no credentials provided,
        // treat this as an anonymous/emulator connection:
        // - don't try to load credentials from env or VM metadata
        // - allow unsigned requests.
        if config.get("endpoint").is_some() && credential.is_none() && credential_path.is_none() {
            builder = builder
                .allow_anonymous()
                .disable_vm_metadata()
                .disable_config_load();
        }
    }

    let op = Operator::new(builder).map_err(CoreError::Storage)?.finish();
    Ok(op)
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
            config: None,
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
            config: None,
        };

        let err = registry.add_source(s).await.unwrap_err();
        assert!(err.to_string().contains("directory does not exist"));
    }
}
