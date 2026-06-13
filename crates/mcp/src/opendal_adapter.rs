use crate::errors::{McpError, McpErrorCode, McpResult};
use crate::registry::StorageRecord;
pub use infimount_core::models::StorageBackendCapabilities;
use infimount_core::{registry, Source, SourceKind};
use opendal::Operator;

pub fn get_capabilities(op: &Operator) -> StorageBackendCapabilities {
    registry::get_capabilities(op)
}

pub fn check_versioning_disabled(storage: &StorageRecord) -> Option<bool> {
    let source = storage_record_to_source(storage).ok()?;
    registry::check_versioning_disabled(&source)
}

pub fn build_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let source = storage_record_to_source(storage)?;
    registry::build_operator(&source).map_err(core_error_to_mcp_error)
}

fn storage_record_to_source(storage: &StorageRecord) -> McpResult<Source> {
    use std::str::FromStr;
    let kind = SourceKind::from_str(&storage.backend).map_err(|_| {
        crate::errors::err_with_details(
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

fn core_error_to_mcp_error(err: infimount_core::CoreError) -> McpError {
    use crate::errors::err_with_details;
    err_with_details(
        McpErrorCode::ERR_INTERNAL,
        err.to_string(),
        serde_json::json!({}),
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

        let op = build_operator(&storage).expect("operator should build");
        let caps = get_capabilities(&op);
        assert!(caps.presign_read);
        assert!(caps.write_with_user_metadata);
    }

    #[test]
    fn unsupported_backend_returns_explicit_error() {
        let err = build_operator(&storage("mystery", json!({}))).unwrap_err();

        assert_eq!(err.code, McpErrorCode::ERR_BACKEND_UNSUPPORTED);
        assert_eq!(err.details["backend"], "mystery");
        assert_eq!(
            check_versioning_disabled(&storage("mystery", json!({}))),
            None
        );
    }

    #[test]
    fn builds_ftp_operator() {
        let op = build_operator(&storage(
            "ftp",
            json!({
                "endpoint": "ftp://example.com:21",
                "user": "alice",
                "password": "password",
                "rootPath": "/workspace",
            }),
        ))
        .expect("operator should build");
        let caps = get_capabilities(&op);
        assert!(!caps.read_with_version);
        assert!(!op.info().full_capability().copy);
        assert!(!caps.presign_read);
    }

    #[cfg(not(windows))]
    #[test]
    fn builds_sftp_operator() {
        let op = build_operator(&storage(
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
        assert!(op.info().full_capability().copy);
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
            let op = build_operator(&storage(backend, config)).expect("operator should build");
            let caps = get_capabilities(&op);
            assert!(op.info().full_capability().copy);
            assert!(op.info().full_capability().rename);
            assert!(!caps.presign_read);
            assert_eq!(op.info().full_capability().list_with_versions, versions);
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
            let op = build_operator(&storage(backend, config)).expect("operator should build");
            let caps = get_capabilities(&op);
            assert!(caps.presign_read);
        }
    }
}
