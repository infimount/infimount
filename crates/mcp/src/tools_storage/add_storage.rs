use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::McpResult;
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

    let storage = ctx.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &name, None)?;
        let mut storage = StorageRecord::new(name.clone(), backend.clone(), input.config.clone());
        storage.enabled = input.enabled;
        storage.mcp_exposed = input.mcp_exposed;
        storage.read_only = input.read_only;
        storages.push(storage.clone());
        Ok(storage)
    })?;

    Ok(AddStorageOutput {
        storage: masked(&storage),
    })
}
