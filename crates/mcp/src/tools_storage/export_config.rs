use serde::{Deserialize, Serialize};

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::registry::{mask_storage_record, StorageRecord};
use crate::tools_fs::FsToolsContext;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportConfigInput {
    #[serde(default)]
    pub include_secrets: bool,
}

#[derive(Debug, Serialize)]
pub struct ExportConfigOutput {
    pub json: String,
}

pub async fn export_config(
    ctx: &FsToolsContext,
    input: ExportConfigInput,
) -> McpResult<ExportConfigOutput> {
    let storages = ctx.registry.load_all()?;
    let exportable: Vec<StorageRecord> = if input.include_secrets {
        storages
    } else {
        storages.iter().map(mask_storage_record).collect()
    };

    let json = serde_json::to_string_pretty(&exportable).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize storage registry",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })?;

    Ok(ExportConfigOutput { json })
}
