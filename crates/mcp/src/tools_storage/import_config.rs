use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    #[serde(default = "default_false")]
    pub mcp_exposed: bool,
    #[serde(default)]
    pub read_only: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
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

pub(crate) fn append_cleanup_journal(account: &str) -> McpResult<()> {
    let path = crate::registry::default_config_dir().join("secret-cleanup.json");
    if let Some(parent) = path.parent() {
        infimount_core::atomic_file::create_dir_all(parent)
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    let start = std::time::Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if start.elapsed() >= std::time::Duration::from_secs(2) => {
                return Err(err(
                    McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                    "timed out acquiring secret cleanup journal lock",
                ));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    let mut document = std::fs::read(&path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
        .unwrap_or_else(|| serde_json::json!({ "pending": [] }));
    let pending = document
        .get_mut("pending")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "secret cleanup journal is invalid",
            )
        })?;
    if !pending
        .iter()
        .any(|item| item.get("account").and_then(Value::as_str) == Some(account))
    {
        pending
            .push(serde_json::json!({ "account": account, "createdAt": Utc::now().to_rfc3339() }));
    }
    let payload = serde_json::to_vec_pretty(&document).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to create cleanup journal",
        )
    })?;
    infimount_core::atomic_file::atomic_write_file(
        &path,
        &payload,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to persist cleanup journal",
        )
    })
}

fn rollback_import_secrets(
    store: &dyn infimount_core::secrets::SecretStore,
    rollback: Vec<(String, Option<Value>)>,
) -> McpResult<()> {
    for (account, previous) in rollback.into_iter().rev() {
        let restored = match previous {
            Some(value) => store.put_json(&account, &value),
            None => store.delete(&account),
        };
        if restored.is_err() {
            return Err(err(
                McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                "credential rollback failed; manual secret-store repair is required",
            ));
        }
    }
    Ok(())
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

    let existing = ctx.registry.load_all()?;
    let current = if input.mode == "replace" {
        Vec::new()
    } else {
        existing.clone()
    };

    let mut merged = current;
    for mut incoming in imported {
        if merged
            .iter()
            .any(|storage| storage.id == incoming.id && storage.name != incoming.name)
        {
            incoming.id = Uuid::new_v4().to_string();
            incoming.secret_ref = None;
            incoming.secret_fields.clear();
        }
        if let Some(idx) = merged
            .iter()
            .position(|storage| storage.name == incoming.name)
        {
            match input.on_conflict.as_str() {
                "error" => {
                    return Err(err_with_details(
                        McpErrorCode::ERR_STORAGE_NAME_CONFLICT,
                        format!("Storage name '{}' already exists", incoming.name),
                        serde_json::json!({ "name": incoming.name }),
                    ))
                }
                "overwrite" => {
                    incoming.id = merged[idx].id.clone();
                    incoming.created_at = merged[idx].created_at.clone();
                    incoming.updated_at = Utc::now().to_rfc3339();
                    incoming.secret_ref = merged[idx].secret_ref.clone();
                    incoming.secret_fields = merged[idx].secret_fields.clone();
                    merged[idx] = incoming;
                }
                "rename" => {
                    incoming.name = next_renamed_name(&merged, &incoming.name);
                    ensure_unique_name(&merged, &incoming.name, None)?;
                    merged.push(incoming);
                }
                _ => unreachable!(),
            }
        } else {
            ensure_unique_name(&merged, &incoming.name, None)?;
            merged.push(incoming);
        }
    }

    let secret_names = infimount_core::secrets::discover_secret_field_names();
    let mut rollback: Vec<(String, Option<Value>)> = Vec::new();
    for storage in &mut merged {
        let extracted =
            infimount_core::secrets::extract_secret_fields(&storage.config, &secret_names);
        infimount_core::secrets::strip_secret_fields(&mut storage.config, &secret_names);
        if extracted.is_empty() {
            continue;
        }
        let previous_account = storage
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{}", storage.id));
        let account = format!("storage/{}/import/{}", storage.id, Uuid::new_v4());
        let previous = ctx
            .registry
            .secret_store()
            .get_json(&previous_account)
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage imported credentials",
                )
            })?;
        if previous.is_none() && (storage.secret_ref.is_some() || !storage.secret_fields.is_empty())
        {
            rollback_import_secrets(ctx.registry.secret_store().as_ref(), rollback)?;
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        let mut bundle = previous.clone().unwrap_or_else(|| serde_json::json!({}));
        let Some(bundle_object) = bundle.as_object_mut() else {
            rollback_import_secrets(ctx.registry.secret_store().as_ref(), rollback)?;
            return Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "stored secret bundle is invalid",
            ));
        };
        bundle_object.extend(extracted.iter().cloned());
        if ctx
            .registry
            .secret_store()
            .put_json(&account, &bundle)
            .is_err()
        {
            let mut current_and_previous = vec![(account.clone(), None)];
            current_and_previous.extend(rollback);
            rollback_import_secrets(ctx.registry.secret_store().as_ref(), current_and_previous)?;
            return Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to import credentials",
            ));
        }
        rollback.push((account.clone(), None));
        storage.secret_ref = Some(account);
        storage.secret_fields = bundle
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
    }
    if let Err(error) = ctx
        .registry
        .save_all_atomic_if_unchanged(&existing, &merged)
    {
        rollback_import_secrets(ctx.registry.secret_store().as_ref(), rollback)?;
        return Err(error);
    }

    let mut warnings = Vec::new();
    let retained_refs = merged
        .iter()
        .filter_map(|storage| storage.secret_ref.as_deref())
        .collect::<std::collections::HashSet<_>>();
    for account in existing
        .iter()
        .filter_map(|storage| storage.secret_ref.as_deref())
        .filter(|account| !retained_refs.contains(account))
    {
        if ctx.registry.secret_store().delete(account).is_err() {
            if append_cleanup_journal(account).is_ok() {
                warnings.push("Credential cleanup is pending and will be retried.".to_string());
            } else {
                warnings.push(
                    "Credential cleanup failed and could not be journaled; remove the native secret-store entry manually."
                        .to_string(),
                );
            }
        }
    }

    Ok(ImportConfigOutput {
        imported: imported_count,
        storages: merged.iter().map(masked).collect(),
        warnings,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingSecretField {
    pub name: String,
    pub storage_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageImportChange {
    pub name: String,
    pub backend: String,
    pub change_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageImportPreview {
    #[serde(rename = "previewId")]
    pub preview_id: String,
    #[serde(rename = "baseRegistryRevision")]
    pub base_registry_revision: String,
    pub additions: Vec<StorageImportChange>,
    pub updates: Vec<StorageImportChange>,
    pub renames: Vec<StorageImportChange>,
    pub removals: Vec<StorageImportChange>,
    pub policy_changes: Vec<StorageImportChange>,
    pub exposure_changes: Vec<StorageImportChange>,
    pub missing_secret_fields: Vec<MissingSecretField>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyStorageImportInput {
    #[serde(rename = "previewId")]
    pub preview_id: String,
    #[serde(rename = "baseRegistryRevision")]
    pub base_registry_revision: String,
    pub mode: String,
    #[serde(rename = "onConflict")]
    pub on_conflict: String,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyStorageImportResult {
    pub applied: usize,
    pub warnings: Vec<String>,
}

struct PreviewEntry {
    #[allow(dead_code)]
    changes: StorageImportPreview,
    storages: Vec<StorageRecord>,
    created_at: Instant,
}

fn preview_store() -> &'static Mutex<HashMap<String, PreviewEntry>> {
    static STORE: OnceLock<Mutex<HashMap<String, PreviewEntry>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_shareable_json(input: &str) -> McpResult<Vec<ImportedStorage>> {
    let value: Value = serde_json::from_str(input).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to parse import JSON",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })?;

    let items = match &value {
        Value::Object(map) => {
            let kind = map.get("kind").and_then(Value::as_str);
            if kind == Some("infimount-shareable-config") {
                map.get("storages")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            "shareable export must have a 'storages' array",
                        )
                    })?
                    .clone()
            } else {
                map.get("storages")
                    .or_else(|| {
                        if map.contains_key("name") || map.contains_key("backend") {
                            Some(&value)
                        } else {
                            None
                        }
                    })
                    .and_then(|v| {
                        if v.is_array() {
                            v.as_array().cloned()
                        } else {
                            Some(vec![v.clone()])
                        }
                    })
                    .ok_or_else(|| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            "import JSON must contain a 'storages' array",
                        )
                    })?
            }
        }
        Value::Array(items) => items.clone(),
        _ => {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "import JSON must be an array or an object",
            ));
        }
    };

    items
        .into_iter()
        .map(|item| {
            #[derive(Deserialize)]
            struct ImportedStorageWire {
                name: String,
                backend: String,
                #[serde(default)]
                config: Value,
                #[serde(default = "default_true")]
                enabled: bool,
                #[serde(default)]
                read_only: bool,
            }
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
                mcp_exposed: false,
                read_only: wire.read_only,
                id: None,
                created_at: None,
                updated_at: None,
            })
        })
        .collect()
}

pub async fn preview_storage_import(
    ctx: &FsToolsContext,
    json: String,
) -> McpResult<StorageImportPreview> {
    let imported = parse_shareable_json(&json)?
        .into_iter()
        .map(ImportedStorage::into_record)
        .collect::<McpResult<Vec<_>>>()?;

    let existing = ctx.registry.load_all()?;
    let existing_by_name: HashMap<&str, &StorageRecord> =
        existing.iter().map(|s| (s.name.as_str(), s)).collect();

    let mut additions = Vec::new();
    let mut updates = Vec::new();
    let mut removals = Vec::new();
    let mut policy_changes = Vec::new();
    let mut exposure_changes = Vec::new();
    let mut missing_secret_fields = Vec::new();
    let mut warnings = Vec::new();

    let secret_names = infimount_core::secrets::discover_secret_field_names();

    for incoming in &imported {
        if let Some(existing_record) = existing_by_name.get(incoming.name.as_str()) {
            let mut update_desc = format!("config updated");
            if incoming.enabled != existing_record.enabled {
                update_desc = format!("{update_desc}, enabled changed");
            }
            updates.push(StorageImportChange {
                name: incoming.name.clone(),
                backend: incoming.backend.clone(),
                change_type: update_desc,
            });
            let has_plaintext = infimount_core::secrets::contains_plaintext_secrets(
                &incoming.config,
                &secret_names,
            );
            if has_plaintext {
                missing_secret_fields.push(MissingSecretField {
                    name: "plaintext credentials in config".into(),
                    storage_name: incoming.name.clone(),
                });
            }
        } else {
            additions.push(StorageImportChange {
                name: incoming.name.clone(),
                backend: incoming.backend.clone(),
                change_type: "new storage".into(),
            });
        }
    }

    for existing_record in &existing {
        if !imported.iter().any(|i| i.name == existing_record.name) {
            removals.push(StorageImportChange {
                name: existing_record.name.clone(),
                backend: existing_record.backend.clone(),
                change_type: "not in import".into(),
            });
        }
    }

    let preview_id = Uuid::new_v4().to_string();
    let base_revision = existing
        .iter()
        .map(|s| s.revision)
        .max()
        .unwrap_or(0)
        .to_string();

    let preview = StorageImportPreview {
        preview_id: preview_id.clone(),
        base_registry_revision: base_revision,
        additions,
        updates,
        renames: Vec::new(),
        removals,
        policy_changes,
        exposure_changes,
        missing_secret_fields,
        warnings,
    };

    let mut store = preview_store().lock().unwrap();
    store.insert(
        preview_id.clone(),
        PreviewEntry {
            changes: preview.clone(),
            storages: imported,
            created_at: Instant::now(),
        },
    );
    store.retain(|_, entry| entry.created_at.elapsed().as_secs() < 600);

    Ok(preview)
}

pub async fn apply_storage_import(
    ctx: &FsToolsContext,
    input: ApplyStorageImportInput,
) -> McpResult<ApplyStorageImportResult> {
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
    if input.mode == "replace" && !input.confirmed {
        return Err(err(
            McpErrorCode::ERR_IMPORT_CONFIRMATION_REQUIRED,
            "replace mode requires explicit confirmation",
        ));
    }

    let entry = {
        let mut store = preview_store().lock().unwrap();
        store.remove(&input.preview_id).ok_or_else(|| {
            err(
                McpErrorCode::ERR_IMPORT_PREVIEW_EXPIRED,
                "import preview not found or expired; re-preview the import",
            )
        })?
    };

    let existing = ctx.registry.load_all()?;
    let current_revision = existing
        .iter()
        .map(|s| s.revision)
        .max()
        .unwrap_or(0)
        .to_string();
    if current_revision != input.base_registry_revision {
        return Err(err(
            McpErrorCode::ERR_IMPORT_PREVIEW_STALE,
            "storage registry has changed since preview; re-preview the import",
        ));
    }

    let imported = entry.storages;
    let imported_count = imported.len();
    let merged = if input.mode == "replace" {
        imported
    } else {
        let mut merged = existing.clone();
        for incoming in imported {
            if let Some(idx) = merged
                .iter()
                .position(|s| s.name == incoming.name)
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
                        let mut record = incoming;
                        record.id = merged[idx].id.clone();
                        record.created_at = merged[idx].created_at.clone();
                        record.updated_at = Utc::now().to_rfc3339();
                        record.secret_ref = merged[idx].secret_ref.clone();
                        record.secret_fields = merged[idx].secret_fields.clone();
                        merged[idx] = record;
                    }
                    "rename" => {
                        let mut record = incoming;
                        record.name = next_renamed_name(&merged, &record.name);
                        ensure_unique_name(&merged, &record.name, None)?;
                        merged.push(record);
                    }
                    _ => unreachable!(),
                }
            } else {
                ensure_unique_name(&merged, &incoming.name, None)?;
                merged.push(incoming);
            }
        }
        merged
    };

    let backup = serde_json::to_vec(&existing).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to create pre-import backup",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })?;

    let result = ctx.registry.save_all_atomic_if_unchanged(&existing, &merged);
    match result {
        Ok(()) => Ok(ApplyStorageImportResult {
            applied: imported_count,
            warnings: Vec::new(),
        }),
        Err(error) => {
            let _ = std::fs::write(
                crate::registry::default_config_dir().join("import-backup.json"),
                &backup,
            );
            Err(error)
        }
    }
}
