use serde::{Deserialize, Serialize};

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::tools_fs::FsToolsContext;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveStorageInput {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveStorageOutput {
    pub removed: bool,
}

pub async fn remove_storage(
    ctx: &FsToolsContext,
    input: RemoveStorageInput,
) -> McpResult<RemoveStorageOutput> {
    ctx.registry.with_locked_mutation(|storages| {
        let before = storages.len();
        storages.retain(|storage| storage.name != input.name);
        if storages.len() == before {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("Storage '{}' not found", input.name),
                serde_json::json!({ "name": input.name }),
            ));
        }
        Ok(())
    })?;

    Ok(RemoveStorageOutput { removed: true })
}
