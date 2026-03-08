use serde::{Deserialize, Serialize};
use tokio::time::{timeout, Duration};

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::tools_fs::FsToolsContext;

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
    let op = opendal_adapter::build_operator(&storage)?;
    let caps = op.info().full_capability();

    let validation = timeout(Duration::from_secs(10), async {
        if caps.list {
            let _ = op.lister("").await?;
        } else {
            let _ = op.stat("").await?;
        }
        Ok::<(), opendal::Error>(())
    })
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
        Ok(Err(err)) => Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "storage validation failed",
            serde_json::json!({ "backend_error": err.to_string() }),
        )),
        Err(_) => Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "storage validation timed out",
            serde_json::json!({ "timeout_seconds": 10 }),
        )),
    }
}
