use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{err, McpErrorCode, McpResult};
use crate::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use crate::tools_fs::FsToolsContext;

use super::common::{canonical_backend, ensure_config_object, masked};

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddStorageInput {
    pub name: String,
    pub backend: String,
    pub config: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_false")]
    pub mcp_exposed: bool,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Serialize)]
pub struct AddStorageOutput {
    pub storage: StorageRecord,
}

pub async fn add_storage(
    ctx: &FsToolsContext,
    input: AddStorageInput,
) -> McpResult<AddStorageOutput> {
    let name = validate_storage_name(&input.name)?;
    let backend = canonical_backend(&input.backend)?;
    ensure_config_object(&input.config)?;

    let mut storage = StorageRecord::new(name.clone(), backend.clone(), input.config.clone());
    storage.enabled = input.enabled;
    storage.mcp_exposed = input.mcp_exposed;
    storage.read_only = input.read_only;
    let secret_names = infimount_core::secrets::discover_secret_field_names();
    let extracted = infimount_core::secrets::extract_secret_fields(&storage.config, &secret_names);
    infimount_core::secrets::strip_secret_fields(&mut storage.config, &secret_names);
    let account = format!("storage/{}", storage.id);
    if !extracted.is_empty() {
        let bundle = serde_json::Value::Object(extracted.iter().cloned().collect());
        ctx.registry
            .secret_store()
            .put_json(&account, &bundle)
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to store credentials",
                )
            })?;
        storage.secret_ref = Some(account.clone());
        storage.secret_fields = extracted.into_iter().map(|(field, _)| field).collect();
    }
    let result = ctx.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &name, None)?;
        storages.push(storage.clone());
        Ok(storage.clone())
    });
    let storage = match result {
        Ok(storage) => storage,
        Err(error) => {
            if storage.secret_ref.is_some() && ctx.registry.secret_store().delete(&account).is_err()
            {
                super::import_config::append_cleanup_journal(&account).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "credential rollback failed and could not be journaled",
                    )
                })?;
            }
            return Err(error);
        }
    };

    Ok(AddStorageOutput {
        storage: masked(&storage),
    })
}
