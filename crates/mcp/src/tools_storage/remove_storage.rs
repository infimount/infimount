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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub async fn remove_storage(
    ctx: &FsToolsContext,
    input: RemoveStorageInput,
) -> McpResult<RemoveStorageOutput> {
    let mut secret_ref = None;
    ctx.registry.with_locked_mutation(|storages| {
        secret_ref = storages
            .iter()
            .find(|storage| storage.name == input.name)
            .and_then(|storage| storage.secret_ref.clone());
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
    let mut warning = None;
    if let Some(account) = secret_ref {
        if ctx.registry.secret_store().delete(&account).is_err() {
            warning = Some(
                if super::import_config::append_cleanup_journal(&account).is_ok() {
                    "Credential cleanup is pending and will be retried."
                } else {
                    "Credential cleanup failed and could not be journaled; remove the native secret-store entry manually."
                }
                .to_string(),
            );
        }
    }

    Ok(RemoveStorageOutput {
        removed: true,
        warning,
    })
}
