use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::registry::StorageRecord;
use opendal::services::{Azblob, Cos, Fs, Gcs, Obs, Oss, Webdav, B2, S3};
use opendal::Operator;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageBackendCapabilities {
    pub list_with_versions: bool,
    pub read_with_version: bool,
    pub delete_with_version: bool,
    pub presign_read: bool,
    pub versioning_disabled: bool,
    pub write_with_user_metadata: bool,
}

pub fn get_capabilities(op: &Operator) -> StorageBackendCapabilities {
    let info = op.info();
    let full = info.full_capability();
    StorageBackendCapabilities {
        list_with_versions: full.list_with_versions,
        read_with_version: full.read_with_version,
        delete_with_version: full.delete_with_version,
        presign_read: full.presign_read,
        versioning_disabled: false,
        write_with_user_metadata: full.write_with_user_metadata,
    }
}

pub fn check_versioning_disabled(storage: &StorageRecord) -> Option<bool> {
    match storage.backend.as_str() {
        "s3" => storage
            .config
            .get("versioning")
            .and_then(|v| v.as_bool())
            .map(|enabled| !enabled),
        "azblob" | "azure_blob" => storage
            .config
            .get("versioning")
            .and_then(|v| v.as_bool())
            .map(|enabled| !enabled),
        "gcs" | "oss" | "aliyun_oss" | "cos" | "tencent_cos" | "obs" | "huawei_obs" => storage
            .config
            .get("versioning")
            .and_then(|v| v.as_bool())
            .map(|enabled| !enabled),
        _ => None,
    }
}

pub fn build_operator(storage: &StorageRecord) -> McpResult<Operator> {
    match storage.backend.as_str() {
        "local" | "fs" => build_fs_operator(storage),
        "s3" => build_s3_operator(storage),
        "webdav" => build_webdav_operator(storage),
        "azure_blob" | "azblob" => build_azblob_operator(storage),
        "gcs" => build_gcs_operator(storage),
        "b2" | "backblaze_b2" => build_b2_operator(storage),
        "oss" | "aliyun_oss" => build_oss_operator(storage),
        "cos" | "tencent_cos" => build_cos_operator(storage),
        "obs" | "huawei_obs" => build_obs_operator(storage),
        other => Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{other}'"),
            serde_json::json!({ "backend": other }),
        )),
    }
}

fn build_fs_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let root = storage
        .config
        .get("root")
        .and_then(|v| v.as_str())
        .or_else(|| storage.config.get("rootPath").and_then(|v| v.as_str()))
        .or_else(|| storage.config.get("path").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "local backend requires config.root or config.path",
                serde_json::json!({ "storage": storage.name }),
            )
        })?;

    let builder = Fs::default().root(&expand_home_path(root));
    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_s3_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = S3::default();

    if let Some(bucket) = storage.config.get("bucket").and_then(|v| v.as_str()) {
        builder = builder.bucket(bucket);
    }
    if let Some(region) = storage.config.get("region").and_then(|v| v.as_str()) {
        builder = builder.region(region);
    }
    if let Some(ak) = storage.config.get("accessKeyId").and_then(|v| v.as_str()) {
        builder = builder.access_key_id(ak);
    }
    if let Some(sk) = storage
        .config
        .get("secretAccessKey")
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_access_key(sk);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(default_acl) = storage.config.get("defaultAcl").and_then(|v| v.as_str()) {
        if !default_acl.trim().is_empty() {
            builder = builder.default_acl(default_acl);
        }
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_webdav_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Webdav::default();

    if let Some(endpoint) = storage
        .config
        .get("serverUrl")
        .or_else(|| storage.config.get("endpoint"))
        .and_then(|v| v.as_str())
    {
        builder = builder.endpoint(endpoint);
    }
    if let Some(username) = storage.config.get("username").and_then(|v| v.as_str()) {
        builder = builder.username(username);
    }
    if let Some(password) = storage.config.get("password").and_then(|v| v.as_str()) {
        builder = builder.password(password);
    }
    if let Some(root) = storage.config.get("rootPath").and_then(|v| v.as_str()) {
        builder = builder.root(root);
    }
    if config_bool(storage, "disableCreateDir").unwrap_or(false) {
        builder = builder.disable_create_dir(true);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_azblob_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Azblob::default();

    if let Some(container) = storage
        .config
        .get("container")
        .or_else(|| storage.config.get("containerName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.container(container);
    }
    if let Some(account_name) = storage.config.get("accountName").and_then(|v| v.as_str()) {
        builder = builder.account_name(account_name);
    }
    if let Some(account_key) = storage.config.get("accountKey").and_then(|v| v.as_str()) {
        builder = builder.account_key(account_key);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_gcs_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Gcs::default();

    if let Some(bucket) = storage
        .config
        .get("bucket")
        .or_else(|| storage.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(root) = storage.config.get("root").and_then(|v| v.as_str()) {
        builder = builder.root(root);
    }
    if let Some(key) = storage
        .config
        .get("credential")
        .and_then(|v| v.as_str())
        .or_else(|| {
            storage
                .config
                .get("serviceAccountJson")
                .and_then(|v| v.as_str())
        })
    {
        builder = builder.credential(key);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_b2_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = B2::default();

    if let Some(bucket) = storage
        .config
        .get("bucket")
        .or_else(|| storage.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(bucket_id) = storage.config.get("bucketId").and_then(|v| v.as_str()) {
        builder = builder.bucket_id(bucket_id);
    }
    if let Some(application_key_id) = storage
        .config
        .get("applicationKeyId")
        .or_else(|| storage.config.get("keyId"))
        .or_else(|| storage.config.get("application_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.application_key_id(application_key_id);
    }
    if let Some(application_key) = storage
        .config
        .get("applicationKey")
        .or_else(|| storage.config.get("application_key"))
        .and_then(|v| v.as_str())
    {
        builder = builder.application_key(application_key);
    }
    if let Some(root) = storage
        .config
        .get("rootPath")
        .or_else(|| storage.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_oss_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Oss::default();

    if let Some(bucket) = storage
        .config
        .get("bucket")
        .or_else(|| storage.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(access_key_id) = storage
        .config
        .get("accessKeyId")
        .or_else(|| storage.config.get("access_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_id(access_key_id);
    }
    if let Some(access_key_secret) = storage
        .config
        .get("accessKeySecret")
        .or_else(|| storage.config.get("access_key_secret"))
        .or_else(|| storage.config.get("secretAccessKey"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_secret(access_key_secret);
    }
    if let Some(root) = storage
        .config
        .get("rootPath")
        .or_else(|| storage.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(addressing_style) = storage
        .config
        .get("addressingStyle")
        .and_then(|v| v.as_str())
    {
        builder = builder.addressing_style(addressing_style);
    }
    if let Some(presign_endpoint) = storage
        .config
        .get("presignEndpoint")
        .and_then(|v| v.as_str())
    {
        builder = builder.presign_endpoint(presign_endpoint);
    }
    if let Some(enabled) = config_bool(storage, "versioning") {
        builder = builder.enable_versioning(enabled);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_cos_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Cos::default();

    if let Some(bucket) = storage
        .config
        .get("bucket")
        .or_else(|| storage.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(secret_id) = storage
        .config
        .get("secretId")
        .or_else(|| storage.config.get("secret_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_id(secret_id);
    }
    if let Some(secret_key) = storage
        .config
        .get("secretKey")
        .or_else(|| storage.config.get("secret_key"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_key(secret_key);
    }
    if let Some(root) = storage
        .config
        .get("rootPath")
        .or_else(|| storage.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(enabled) = config_bool(storage, "versioning") {
        builder = builder.enable_versioning(enabled);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn build_obs_operator(storage: &StorageRecord) -> McpResult<Operator> {
    let mut builder = Obs::default();

    if let Some(bucket) = storage
        .config
        .get("bucket")
        .or_else(|| storage.config.get("bucketName"))
        .and_then(|v| v.as_str())
    {
        builder = builder.bucket(bucket);
    }
    if let Some(endpoint) = storage.config.get("endpoint").and_then(|v| v.as_str()) {
        builder = builder.endpoint(endpoint);
    }
    if let Some(access_key_id) = storage
        .config
        .get("accessKeyId")
        .or_else(|| storage.config.get("access_key_id"))
        .and_then(|v| v.as_str())
    {
        builder = builder.access_key_id(access_key_id);
    }
    if let Some(secret_access_key) = storage
        .config
        .get("secretAccessKey")
        .or_else(|| storage.config.get("secret_access_key"))
        .or_else(|| storage.config.get("accessKeySecret"))
        .and_then(|v| v.as_str())
    {
        builder = builder.secret_access_key(secret_access_key);
    }
    if let Some(root) = storage
        .config
        .get("rootPath")
        .or_else(|| storage.config.get("root"))
        .and_then(|v| v.as_str())
    {
        builder = builder.root(root);
    }
    if let Some(enabled) = config_bool(storage, "versioning") {
        builder = builder.enable_versioning(enabled);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}

fn config_bool(storage: &StorageRecord, key: &str) -> Option<bool> {
    match storage.config.get(key)? {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Some(true),
            "false" | "0" | "no" | "n" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn expand_home_path(input: &str) -> String {
    if input == "~" {
        return std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| input.to_string());
    }

    if let Some(rest) = input.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if !home.is_empty() {
            return format!("{home}/{rest}");
        }
    }

    input.to_string()
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

    #[test]
    fn config_bool_accepts_boolean_and_string_values() {
        let storage = storage(
            "webdav",
            json!({
                "disableCreateDir": true,
                "legacyFlag": "off"
            }),
        );

        assert_eq!(config_bool(&storage, "disableCreateDir"), Some(true));
        assert_eq!(config_bool(&storage, "legacyFlag"), Some(false));
        assert_eq!(config_bool(&storage, "missing"), None);
    }
}
