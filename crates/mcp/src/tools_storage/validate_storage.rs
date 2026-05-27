use opendal::ErrorKind;
use serde::{Deserialize, Serialize};
use std::fs;
use tokio::time::{timeout, Duration};

use crate::errors::McpResult;
use crate::opendal_adapter;
use crate::registry::StorageRecord;
use crate::tools_fs::FsToolsContext;

const VALIDATE_STORAGE_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateStorageInput {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct StorageCapabilities {
    pub list: bool,
    pub stat: bool,
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub copy: bool,
    pub rename: bool,
    pub presign_read: bool,
    pub create_dir: bool,
    pub write_with_user_metadata: bool,
    pub list_with_versions: bool,
    pub read_with_version: bool,
    pub delete_with_version: bool,
}

#[derive(Debug, Serialize)]
pub struct ValidateStorageOutput {
    pub valid: bool,
    pub details: String,
    pub capabilities: StorageCapabilities,
    pub fix_hints: Vec<String>,
    pub warnings: Vec<String>,
}

pub async fn validate_storage(
    ctx: &FsToolsContext,
    input: ValidateStorageInput,
) -> McpResult<ValidateStorageOutput> {
    let storage = ctx.registry.find_by_name(&input.name)?;
    validate_storage_record(&storage).await
}

pub async fn validate_storage_record(storage: &StorageRecord) -> McpResult<ValidateStorageOutput> {
    let op = opendal_adapter::build_operator(storage)?;
    let caps = op.info().full_capability();
    let capabilities = storage_capabilities(storage, &op);
    let warnings = validation_warnings(storage, &capabilities);

    if matches!(storage.backend.as_str(), "local" | "fs") {
        let root = storage
            .config
            .get("root")
            .and_then(|value| value.as_str())
            .or_else(|| {
                storage
                    .config
                    .get("rootPath")
                    .and_then(|value| value.as_str())
            })
            .or_else(|| storage.config.get("path").and_then(|value| value.as_str()));

        if let Some(root) = root {
            let is_valid_dir = fs::metadata(root)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !is_valid_dir {
                return Ok(ValidateStorageOutput {
                    valid: false,
                    details: "local root is not an existing directory".to_string(),
                    capabilities,
                    fix_hints: vec![
                        "Choose an existing folder for the local storage root.".to_string()
                    ],
                    warnings,
                });
            }
        }
    }

    let validation = timeout(
        Duration::from_secs(VALIDATE_STORAGE_TIMEOUT_SECONDS),
        async {
            if caps.list {
                let _ = op.lister("").await?;
            } else {
                let _ = op.stat("").await?;
            }
            Ok::<(), opendal::Error>(())
        },
    )
    .await;

    match validation {
        Ok(Ok(())) => Ok(ValidateStorageOutput {
            valid: true,
            details: "storage validation succeeded".to_string(),
            capabilities,
            fix_hints: Vec::new(),
            warnings,
        }),
        Ok(Err(err)) => {
            let (details, fix_hints) = classify_validation_error(&err);
            Ok(ValidateStorageOutput {
                valid: false,
                details,
                capabilities,
                fix_hints,
                warnings,
            })
        }
        Err(_) => Ok(ValidateStorageOutput {
            valid: false,
            details: "storage validation timed out".to_string(),
            capabilities,
            fix_hints: vec![
                "Check the endpoint, network connection, credentials, and bucket or container name."
                    .to_string(),
            ],
            warnings,
        }),
    }
}

fn storage_capabilities(storage: &StorageRecord, op: &opendal::Operator) -> StorageCapabilities {
    let caps = op.info().full_capability();
    let mut versioning_caps = opendal_adapter::get_capabilities(op);
    if opendal_adapter::check_versioning_disabled(storage) == Some(true) {
        versioning_caps.list_with_versions = false;
        versioning_caps.read_with_version = false;
        versioning_caps.delete_with_version = false;
    }

    StorageCapabilities {
        list: caps.list,
        stat: caps.stat,
        read: caps.read,
        write: caps.write,
        delete: caps.delete,
        copy: caps.copy,
        rename: caps.rename,
        presign_read: caps.presign_read,
        create_dir: caps.create_dir,
        write_with_user_metadata: caps.write_with_user_metadata,
        list_with_versions: versioning_caps.list_with_versions,
        read_with_version: versioning_caps.read_with_version,
        delete_with_version: versioning_caps.delete_with_version,
    }
}

fn validation_warnings(storage: &StorageRecord, capabilities: &StorageCapabilities) -> Vec<String> {
    let mut warnings = Vec::new();
    if !storage.enabled {
        warnings.push(
            "Storage is disabled and will not be available in the desktop app or MCP.".to_string(),
        );
    }
    if !storage.mcp_exposed {
        warnings.push("Storage is not exposed to MCP clients.".to_string());
    }
    if storage.enabled
        && storage.mcp_exposed
        && !storage.read_only
        && (capabilities.write || capabilities.delete || capabilities.rename || capabilities.copy)
    {
        warnings.push(
            "MCP-exposed storage is writable; review enabled tools, path policy, and confirmations before granting agent access."
                .to_string(),
        );
    }
    if storage.enabled && storage.mcp_exposed && capabilities.presign_read {
        warnings.push(
            "This backend can create presigned download links when the MCP link tool is enabled."
                .to_string(),
        );
    }
    warnings
}

fn classify_validation_error(error: &opendal::Error) -> (String, Vec<String>) {
    match error.kind() {
        ErrorKind::NotFound => (
            "storage root, bucket, container, or prefix was not found".to_string(),
            vec!["Check that the target exists and the configured root or prefix is correct."
                .to_string()],
        ),
        ErrorKind::PermissionDenied => (
            "storage credentials do not have permission to validate this location".to_string(),
            vec!["Check credentials and ensure they allow at least list or stat access.".to_string()],
        ),
        ErrorKind::ConfigInvalid => (
            "storage configuration is invalid".to_string(),
            vec!["Review required fields such as endpoint, bucket or container, region, and credentials."
                .to_string()],
        ),
        ErrorKind::Unsupported => (
            "storage backend does not support the validation operation".to_string(),
            vec!["Review the backend capability summary for unsupported operations.".to_string()],
        ),
        _ => (
            "storage validation failed".to_string(),
            vec!["Check endpoint, network connectivity, credentials, and backend-specific settings."
                .to_string()],
        ),
    }
}
