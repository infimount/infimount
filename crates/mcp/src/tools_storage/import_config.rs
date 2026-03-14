use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{err, err_with_details, McpErrorCode, McpResult};
use crate::registry::{ensure_unique_name, StorageRecord};
use crate::tools_fs::FsToolsContext;

use super::common::{masked, next_renamed_name, ImportedStorage};

fn default_mode() -> String {
    "merge".to_string()
}

fn default_on_conflict() -> String {
    "error".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportConfigInput {
    pub json: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_on_conflict")]
    pub on_conflict: String,
}

#[derive(Debug, Serialize)]
pub struct ImportConfigOutput {
    pub imported: usize,
    pub storages: Vec<StorageRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportedStorageWire {
    pub id: Option<String>,
    pub name: String,
    pub backend: String,
    pub config: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub mcp_exposed: bool,
    #[serde(default)]
    pub read_only: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

fn default_true() -> bool {
    true
}

fn parse_import_json(input: &str) -> McpResult<Vec<ImportedStorage>> {
    let value: Value = serde_json::from_str(input).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to parse import JSON",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })?;

    let items = match value {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("storages") {
            Some(Value::Array(items)) => items,
            _ => {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "import JSON must be an array or an object with a 'storages' array",
                ));
            }
        },
        _ => {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "import JSON must be an array or an object with a 'storages' array",
            ));
        }
    };

    items
        .into_iter()
        .map(|item| {
            let wire: ImportedStorageWire = serde_json::from_value(item).map_err(|e| {
                err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    "imported storage entry is invalid",
                    serde_json::json!({ "serde_error": e.to_string() }),
                )
            })?;

            Ok(ImportedStorage {
                name: wire.name,
                backend: wire.backend,
                config: wire.config,
                enabled: wire.enabled,
                mcp_exposed: wire.mcp_exposed,
                read_only: wire.read_only,
                id: wire.id,
                created_at: wire.created_at,
                updated_at: wire.updated_at,
            })
        })
        .collect()
}

pub async fn import_config(
    ctx: &FsToolsContext,
    input: ImportConfigInput,
) -> McpResult<ImportConfigOutput> {
    if !matches!(input.mode.as_str(), "merge" | "replace") {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "mode must be 'merge' or 'replace'",
        ));
    }
    if !matches!(input.on_conflict.as_str(), "error" | "overwrite" | "rename") {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "on_conflict must be 'error', 'overwrite', or 'rename'",
        ));
    }

    let imported = parse_import_json(&input.json)?
        .into_iter()
        .map(ImportedStorage::into_record)
        .collect::<McpResult<Vec<_>>>()?;
    let imported_count = imported.len();

    let current = if input.mode == "replace" {
        Vec::new()
    } else {
        ctx.registry.load_all()?
    };

    let merged = ctx.registry.with_locked_mutation(|storages| {
        if input.mode == "replace" {
            storages.clear();
        } else {
            *storages = current.clone();
        }

        for mut incoming in imported.clone() {
            if let Some(idx) = storages
                .iter()
                .position(|storage| storage.name == incoming.name)
            {
                match input.on_conflict.as_str() {
                    "error" => {
                        return Err(err_with_details(
                            McpErrorCode::ERR_STORAGE_NAME_CONFLICT,
                            format!("Storage name '{}' already exists", incoming.name),
                            serde_json::json!({ "name": incoming.name }),
                        ));
                    }
                    "overwrite" => {
                        let existing = &storages[idx];
                        incoming.id = existing.id.clone();
                        incoming.created_at = existing.created_at.clone();
                        incoming.updated_at = Utc::now().to_rfc3339();
                        storages[idx] = incoming;
                    }
                    "rename" => {
                        incoming.name = next_renamed_name(storages, &incoming.name);
                        ensure_unique_name(storages, &incoming.name, None)?;
                        storages.push(incoming);
                    }
                    _ => unreachable!(),
                }
            } else {
                ensure_unique_name(storages, &incoming.name, None)?;
                storages.push(incoming);
            }
        }

        Ok(storages.clone())
    })?;

    Ok(ImportConfigOutput {
        imported: imported_count,
        storages: merged.iter().map(masked).collect(),
    })
}
