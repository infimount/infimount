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
}

#[derive(Debug, Serialize)]
pub struct ValidateStorageOutput {
    pub valid: bool,
    pub details: String,
    pub capabilities: StorageCapabilities,
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
                    details: "storage validation failed".to_string(),
                    capabilities: StorageCapabilities {
                        list: caps.list,
                        stat: caps.stat,
                        read: caps.read,
                        write: caps.write,
                        delete: caps.delete,
                        copy: caps.copy,
                        rename: caps.rename,
                        presign_read: caps.presign_read,
                        create_dir: caps.create_dir,
                    },
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
            capabilities: StorageCapabilities {
                list: caps.list,
                stat: caps.stat,
                read: caps.read,
                write: caps.write,
                delete: caps.delete,
                copy: caps.copy,
                rename: caps.rename,
                presign_read: caps.presign_read,
                create_dir: caps.create_dir,
            },
        }),
        Ok(Err(_err)) => Ok(ValidateStorageOutput {
            valid: false,
            details: "storage validation failed".to_string(),
            capabilities: StorageCapabilities {
                list: caps.list,
                stat: caps.stat,
                read: caps.read,
                write: caps.write,
                delete: caps.delete,
                copy: caps.copy,
                rename: caps.rename,
                presign_read: caps.presign_read,
                create_dir: caps.create_dir,
            },
        }),
        Err(_) => Ok(ValidateStorageOutput {
            valid: false,
            details: "storage validation timed out".to_string(),
            capabilities: StorageCapabilities {
                list: caps.list,
                stat: caps.stat,
                read: caps.read,
                write: caps.write,
                delete: caps.delete,
                copy: caps.copy,
                rename: caps.rename,
                presign_read: caps.presign_read,
                create_dir: caps.create_dir,
            },
        }),
    }
}
