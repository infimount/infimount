use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::registry::StorageRecord;
use opendal::services::{Azblob, Fs, Gcs, Webdav, S3};
use opendal::Operator;

pub fn build_operator(storage: &StorageRecord) -> McpResult<Operator> {
    match storage.backend.as_str() {
        "local" | "fs" => build_fs_operator(storage),
        "s3" => build_s3_operator(storage),
        "webdav" => build_webdav_operator(storage),
        "azure_blob" | "azblob" => build_azblob_operator(storage),
        "gcs" => build_gcs_operator(storage),
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
        .or_else(|| storage.config.get("path").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "local backend requires config.root or config.path",
                serde_json::json!({ "storage": storage.name }),
            )
        })?;

    let builder = Fs::default().root(root);
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
        .get("serviceAccountJson")
        .and_then(|v| v.as_str())
    {
        builder = builder.credential(key);
    }

    Operator::new(builder)
        .map_err(|e| super::errors::map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))
        .map(|op| op.finish())
}
