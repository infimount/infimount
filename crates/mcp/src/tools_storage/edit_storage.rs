use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use crate::tools_fs::FsToolsContext;

use super::common::{ensure_backend_supported, ensure_config_object, masked};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditStoragePatch {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub mcp_exposed: Option<bool>,
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub new_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditStorageInput {
    pub name: String,
    pub patch: EditStoragePatch,
}

#[derive(Debug, Serialize)]
pub struct EditStorageOutput {
    pub storage: StorageRecord,
}

pub async fn edit_storage(
    ctx: &FsToolsContext,
    input: EditStorageInput,
) -> McpResult<EditStorageOutput> {
    if let Some(ref backend) = input.patch.backend {
        ensure_backend_supported(backend)?;
    }
    if let Some(ref config) = input.patch.config {
        ensure_config_object(config)?;
    }

    let updated = ctx.registry.with_locked_mutation(|storages| {
        let idx = storages
            .iter()
            .position(|storage| storage.name == input.name)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("Storage '{}' not found", input.name),
                    serde_json::json!({ "name": input.name }),
                )
            })?;

        let mut storage = storages[idx].clone();
        if let Some(ref new_name) = input.patch.new_name {
            let normalized_name = validate_storage_name(new_name)?;
            ensure_unique_name(storages, &normalized_name, Some(storage.id.as_str()))?;
            storage.name = normalized_name;
        }
        if let Some(ref backend) = input.patch.backend {
            storage.backend = backend.clone();
        }
        if let Some(ref config) = input.patch.config {
            storage.config = config.clone();
        }
        if let Some(enabled) = input.patch.enabled {
            storage.enabled = enabled;
        }
        if let Some(mcp_exposed) = input.patch.mcp_exposed {
            storage.mcp_exposed = mcp_exposed;
        }
        if let Some(read_only) = input.patch.read_only {
            storage.read_only = read_only;
        }
        storage.updated_at = Utc::now().to_rfc3339();
        storages[idx] = storage.clone();
        Ok(storage)
    })?;

    Ok(EditStorageOutput {
        storage: masked(&updated),
    })
}
