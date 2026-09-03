use crate::errors::{err_with_details, McpError, McpErrorCode, McpResult};
use crate::registry::{ResolvedStorageRecord, StorageRecord, StorageRegistry};
pub use infimount_core::models::StorageBackendCapabilities;
use infimount_core::runtime::{get_or_create_operator, OperatorCache};
use infimount_core::secrets::SecretStore;
use infimount_core::{registry, Source, SourceKind};
use opendal::Operator;
use std::sync::{Arc, OnceLock};

static MCP_OPERATOR_CACHE: OnceLock<OperatorCache> = OnceLock::new();

pub fn clear_operator_cache() {
    if let Some(cache) = MCP_OPERATOR_CACHE.get() {
        cache.clear();
    }
}

pub fn get_capabilities(op: &Operator) -> StorageBackendCapabilities {
    registry::get_capabilities(op)
}

pub fn check_versioning_disabled(storage: &StorageRecord) -> Option<bool> {
    let source = storage_record_to_source(storage).ok()?;
    registry::check_versioning_disabled(&source)
}

pub fn build_operator(
    storage: &StorageRecord,
    storage_registry: &StorageRegistry,
) -> McpResult<Operator> {
    let cache = MCP_OPERATOR_CACHE.get_or_init(OperatorCache::new);
    if let Some(operator) = cache.get_for_storage(&storage.id, storage.revision) {
        return Ok(operator);
    }
    let resolved = storage_registry.resolve_storage(storage)?;
    build_operator_resolved(&resolved)
}

/// Builds from an ephemeral draft config. Never use this for persisted records.
pub fn build_operator_from_config(storage: &StorageRecord) -> McpResult<Operator> {
    let source = storage_record_to_source(storage)?;
    registry::build_operator(&source).map_err(core_error_to_mcp_error)
}

pub fn build_operator_resolved(resolved: &ResolvedStorageRecord) -> McpResult<Operator> {
    let source = resolved_record_to_source(resolved)?;
    let cache = MCP_OPERATOR_CACHE.get_or_init(OperatorCache::new);
    get_or_create_operator(cache, &source, resolved.record.revision)
        .map_err(core_error_to_mcp_error)
}

pub fn resolve_and_build(
    record: &StorageRecord,
    secret_store: &Arc<dyn SecretStore>,
) -> McpResult<Operator> {
    let registry = StorageRegistry::with_secret_store(None, secret_store.clone());
    let resolved = registry.resolve_storage(record).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
            format!("failed to resolve storage '{}'", record.name),
            serde_json::json!({ "storage_id": record.id }),
        )
    })?;
    let source = resolved_record_to_source(&resolved)?;
    registry::build_operator(&source).map_err(core_error_to_mcp_error)
}

fn storage_record_to_source(storage: &StorageRecord) -> McpResult<Source> {
    use std::str::FromStr;
    let kind = SourceKind::from_str(&storage.backend).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{}'", storage.backend),
            serde_json::json!({ "backend": storage.backend }),
        )
    })?;

    Ok(Source {
        id: storage.id.clone(),
        name: storage.name.clone(),
        kind,
        root: String::new(),
        config: storage.config.clone(),
    })
}

fn resolved_record_to_source(resolved: &ResolvedStorageRecord) -> McpResult<Source> {
    use std::str::FromStr;
    let kind = SourceKind::from_str(&resolved.record.backend).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{}'", resolved.record.backend),
            serde_json::json!({ "backend": resolved.record.backend }),
        )
    })?;

    Ok(Source {
        id: resolved.record.id.clone(),
        name: resolved.record.name.clone(),
        kind,
        root: String::new(),
        config: resolved.resolved_config.clone(),
    })
}

fn core_error_to_mcp_error(_error: infimount_core::CoreError) -> McpError {
    err_with_details(
        McpErrorCode::ERR_INTERNAL,
        "storage backend operation failed",
        serde_json::json!({ "kind": "Unexpected", "temporary": false }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn storage(backend: &str, config: serde_json::Value) -> StorageRecord {
        StorageRecord::new("Test".to_string(), backend.to_string(), config)
    }

    #[test]
    fn resolves_s3_and_oauth_drive_credentials_only_in_memory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let secret_store = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(temp.path().join("storages.json")),
            secret_store.clone(),
        );
        for (backend, public, secret) in [
            (
                "s3",
                json!({ "bucket": "example", "region": "us-east-1" }),
                json!({ "accessKeyId": "key-id", "secretAccessKey": "key-secret" }),
            ),
            (
                "gdrive",
                json!({ "rootPath": "/" }),
                json!({ "clientId": "client-id", "accessToken": "access-token" }),
            ),
            (
                "onedrive",
                json!({ "rootPath": "/", "versioning": false }),
                json!({ "clientId": "client-id", "accessToken": "access-token" }),
            ),
        ] {
            let mut record = storage(backend, public);
            let account = format!("storage/{}", record.id);
            secret_store
                .put_json(&account, &secret)
                .expect("store secret");
            record.secret_ref = Some(account);
            let operator = build_operator(&record, &registry).expect("resolve operator");
            assert!(!operator.info().scheme().to_string().is_empty());
            assert!(record.config.get("accessToken").is_none());
            assert!(record.config.get("secretAccessKey").is_none());
        }
    }

    #[test]
    fn persisted_cache_hit_does_not_reopen_secret_store_and_revision_miss_fails_closed() {
        clear_operator_cache();
        let temp = tempfile::tempdir().expect("temp dir");
        let secret_store = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(temp.path().join("storages.json")),
            secret_store.clone(),
        );
        let mut record = storage("s3", json!({ "bucket": "example", "region": "us-east-1" }));
        let account = format!("storage/{}", record.id);
        secret_store
            .put_json(
                &account,
                &json!({ "accessKeyId": "id", "secretAccessKey": "secret" }),
            )
            .unwrap();
        record.secret_ref = Some(account.clone());

        build_operator(&record, &registry).expect("initial cache population");
        secret_store.delete(&account).unwrap();
        build_operator(&record, &registry).expect("same revision must use cache");

        record.revision += 1;
        let error =
            build_operator(&record, &registry).expect_err("new revision must resolve again");
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_NOT_FOUND);
        clear_operator_cache();
    }

    #[test]
    fn missing_secret_bundle_fails_closed() {
        let temp = tempfile::tempdir().expect("temp dir");
        let registry = StorageRegistry::with_secret_store(
            Some(temp.path().join("storages.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let mut record = storage("s3", json!({ "bucket": "example" }));
        record.secret_ref = Some(format!("storage/{}", record.id));
        let error = build_operator(&record, &registry).expect_err("missing bundle should fail");
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_NOT_FOUND);
    }

    #[test]
    fn builds_native_b2_operator_and_reports_metadata_capability() {
        let storage = storage(
            "b2",
            json!({
                "bucket": "bucket-name",
                "bucketId": "bucket-id",
                "applicationKeyId": "key-id",
                "applicationKey": "application-key",
                "rootPath": "/workspace"
            }),
        );

        let op = build_operator_from_config(&storage).expect("operator should build");
        let caps = get_capabilities(&op);
        assert!(caps.presign_read);
        assert!(caps.write_with_user_metadata);
    }

    #[test]
    fn unsupported_backend_returns_explicit_error() {
        let err = build_operator_from_config(&storage("mystery", json!({}))).unwrap_err();

        assert_eq!(err.code, McpErrorCode::ERR_BACKEND_UNSUPPORTED);
        assert_eq!(err.details["backend"], "mystery");
        assert_eq!(
            check_versioning_disabled(&storage("mystery", json!({}))),
            None
        );
    }

    #[test]
    fn ftp_backend_is_disabled() {
        let err = build_operator_from_config(&storage("ftp", json!({}))).unwrap_err();
        assert_eq!(err.code, McpErrorCode::ERR_INTERNAL);
    }

    #[cfg(not(windows))]
    #[test]
    fn builds_sftp_operator() {
        let op = build_operator_from_config(&storage(
            "sftp",
            json!({
                "endpoint": "ssh://example.com:22",
                "user": "alice",
                "privateKeyPath": "/home/alice/.ssh/id_ed25519",
                "rootPath": "/workspace",
                "enableCopy": true,
            }),
        ))
        .expect("operator should build");
        let caps = get_capabilities(&op);
        assert!(!caps.read_with_version);
        assert!(op.info().capability().copy);
        assert!(!caps.presign_read);
    }

    #[test]
    fn builds_oauth_drive_operators() {
        for (backend, config, versions) in [
            (
                "gdrive",
                json!({
                    "refreshToken": "refresh-token",
                    "clientId": "client-id",
                    "clientSecret": "client-secret",
                    "rootPath": "/workspace"
                }),
                false,
            ),
            (
                "onedrive",
                json!({
                    "refreshToken": "refresh-token",
                    "clientId": "client-id",
                    "rootPath": "/workspace",
                    "versioning": true
                }),
                true,
            ),
        ] {
            let op = build_operator_from_config(&storage(backend, config))
                .expect("operator should build");
            let caps = get_capabilities(&op);
            assert!(op.info().capability().copy);
            assert!(op.info().capability().rename);
            assert!(!caps.presign_read);
            assert_eq!(op.info().capability().list_with_versions, versions);
        }
    }

    #[test]
    fn builds_v0_7_object_store_operators() {
        for (backend, config) in [
            (
                "oss",
                json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://oss-cn-beijing.aliyuncs.com",
                    "accessKeyId": "key-id",
                    "accessKeySecret": "key-secret",
                    "rootPath": "/workspace"
                }),
            ),
            (
                "cos",
                json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://cos.ap-singapore.myqcloud.com",
                    "secretId": "secret-id",
                    "secretKey": "secret-key",
                    "rootPath": "/workspace"
                }),
            ),
            (
                "obs",
                json!({
                    "bucket": "bucket-name",
                    "endpoint": "https://obs.cn-north-4.myhuaweicloud.com",
                    "accessKeyId": "key-id",
                    "secretAccessKey": "key-secret",
                    "rootPath": "/workspace"
                }),
            ),
        ] {
            let op = build_operator_from_config(&storage(backend, config))
                .expect("operator should build");
            let caps = get_capabilities(&op);
            assert!(caps.presign_read);
        }
    }
}
