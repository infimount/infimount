use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::{err, err_with_details, sanitized_parse_error, McpErrorCode, McpResult};
use crate::policy::{normalize_storage_policy, McpStoragePolicy};
use crate::registry::{
    ensure_unique_name, ImportTransactionJournal, ImportTransactionState, StorageRecord,
    IMPORT_JOURNAL_VERSION,
};
use crate::tools_fs::FsToolsContext;

#[cfg(test)]
use super::common::masked;
use super::common::{next_renamed_name, ImportedStorage};

fn default_mode() -> String {
    "merge".to_string()
}

fn default_on_conflict() -> String {
    "error".to_string()
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportConfigInput {
    pub json: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_on_conflict")]
    pub on_conflict: String,
}

#[cfg(test)]
#[derive(Debug, Serialize)]
pub struct ImportConfigOutput {
    pub imported: usize,
    pub storages: Vec<StorageRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[cfg(test)]
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

#[cfg(test)]
fn default_false() -> bool {
    false
}

fn assign_import_revisions(
    existing: &[StorageRecord],
    merged: &mut [StorageRecord],
) -> McpResult<()> {
    for record in merged {
        let Some(previous) = existing.iter().find(|item| item.id == record.id) else {
            record.revision = record.revision.max(1);
            continue;
        };
        let mut comparable = record.clone();
        comparable.revision = previous.revision;
        let unchanged = serde_json::to_value(&comparable).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to compare imported storage revision",
            )
        })? == serde_json::to_value(previous).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to compare existing storage revision",
            )
        })?;
        record.revision = if unchanged {
            previous.revision
        } else {
            previous.revision.saturating_add(1)
        };
    }
    Ok(())
}

#[cfg(test)]
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

pub(crate) fn append_cleanup_journal_at(
    registry_path: &std::path::Path,
    account: &str,
) -> McpResult<()> {
    crate::registry::append_secret_cleanup_at(registry_path, account)
}

pub(crate) fn append_cleanup_journal(account: &str) -> McpResult<()> {
    append_cleanup_journal_at(&crate::registry::default_registry_path(), account)
}

fn rollback_import_secrets(
    store: &dyn infimount_core::secrets::SecretStore,
    rollback: Vec<(String, Option<Value>)>,
) -> McpResult<()> {
    let mut failures = Vec::new();
    for (account, previous) in rollback.into_iter().rev() {
        let restored = match previous {
            Some(value) => store.put_json(&account, &value),
            None => store.delete(&account),
        };
        if restored.is_err() {
            failures.push(account);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(err_with_details(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "one or more credential rollback stages failed; manual secret-store repair is required",
            serde_json::json!({ "failedAccounts": failures }),
        ))
    }
}

#[cfg(test)]
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
        infimount_core::secrets::canonicalize_bundle_keys(&mut bundle);
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
    if let Err(error) = assign_import_revisions(&existing, &mut merged) {
        rollback_import_secrets(ctx.registry.secret_store().as_ref(), rollback)?;
        return Err(error);
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
            if append_cleanup_journal_at(ctx.registry.path(), account).is_ok() {
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
    /// RFC 6901 JSON Pointer identifying the missing credential field.
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
    pub mode: String,
    #[serde(rename = "onConflict")]
    pub on_conflict: String,
    pub additions: Vec<StorageImportChange>,
    pub updates: Vec<StorageImportChange>,
    pub renames: Vec<StorageImportChange>,
    pub removals: Vec<StorageImportChange>,
    pub policy_changes: Vec<StorageImportChange>,
    pub exposure_changes: Vec<StorageImportChange>,
    pub missing_secret_fields: Vec<MissingSecretField>,
    pub warnings: Vec<String>,
    #[serde(rename = "requiresConfirmation")]
    pub requires_confirmation: bool,
    #[serde(rename = "confirmationReasons")]
    pub confirmation_reasons: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewStorageImportInput {
    pub json: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_on_conflict")]
    pub on_conflict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyStorageImportInput {
    #[serde(rename = "previewId")]
    pub preview_id: String,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyStorageImportResult {
    pub applied: usize,
    pub warnings: Vec<String>,
}

const IMPORT_PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);
pub(super) const IMPORT_PREVIEW_MAX_ENTRIES: usize = 32;

#[derive(Clone)]
struct SecretStage {
    record_id: String,
    explicit: serde_json::Map<String, Value>,
}

impl SecretStage {
    fn zeroize(&mut self) {
        for value in self.explicit.values_mut() {
            zeroize_json_value(value);
        }
    }
}

#[derive(Clone)]
struct PreviewEntry {
    changes: StorageImportPreview,
    storages: Vec<StorageRecord>,
    secret_stages: Vec<SecretStage>,
    base_registry_snapshot: Vec<u8>,
    created_at: Instant,
}

impl PreviewEntry {
    fn zeroize(&mut self) {
        for stage in &mut self.secret_stages {
            stage.zeroize();
        }
    }
}

impl Drop for PreviewEntry {
    fn drop(&mut self) {
        self.zeroize();
    }
}

fn zeroize_json_value(value: &mut Value) {
    use zeroize::Zeroize;
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => {
            for item in items {
                zeroize_json_value(item);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                zeroize_json_value(value);
            }
        }
        _ => {}
    }
}

fn ensure_preview_expiry_task() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        tokio::spawn(async {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await;
            loop {
                interval.tick().await;
                let mut store = preview_store().lock().unwrap();
                store.retain(|_, entry| {
                    let keep = entry.created_at.elapsed() < IMPORT_PREVIEW_TTL;
                    if !keep {
                        entry.zeroize();
                    }
                    keep
                });
            }
        });
    });
}

fn insert_preview_entry(preview_id: String, entry: PreviewEntry) {
    let mut store = preview_store().lock().unwrap();
    store.retain(|_, existing| {
        let keep = existing.created_at.elapsed() < IMPORT_PREVIEW_TTL;
        if !keep {
            existing.zeroize();
        }
        keep
    });
    if store.len() >= IMPORT_PREVIEW_MAX_ENTRIES {
        let oldest = store
            .iter()
            .min_by_key(|(_, existing)| existing.created_at)
            .map(|(id, _)| id.clone());
        if let Some(oldest) = oldest {
            if let Some(mut evicted) = store.remove(&oldest) {
                evicted.zeroize();
            }
        }
    }
    store.insert(preview_id, entry);
}

fn remove_preview_entry_zeroized(preview_id: &str) {
    if let Some(mut entry) = preview_store().lock().unwrap().remove(preview_id) {
        entry.zeroize();
    }
}

fn preview_store() -> &'static Mutex<HashMap<String, PreviewEntry>> {
    static STORE: OnceLock<Mutex<HashMap<String, PreviewEntry>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Explicitly cancel a pending import preview and zeroize any staged secrets.
pub fn cancel_storage_import_preview(preview_id: &str) -> McpResult<()> {
    let removed = preview_store().lock().unwrap().remove(preview_id);
    match removed {
        Some(mut entry) => {
            entry.zeroize();
            Ok(())
        }
        None => Err(err(
            McpErrorCode::ERR_INTERNAL,
            "import preview not found; it may have expired",
        )),
    }
}

/// Zeroize every pending import preview. Intended for application shutdown.
pub fn zeroize_all_storage_import_previews() {
    let mut store = preview_store().lock().unwrap();
    for (_, entry) in store.iter_mut() {
        entry.zeroize();
    }
    store.clear();
}

#[cfg(test)]
pub(super) fn expire_storage_import_preview(preview_id: &str) {
    if let Some(entry) = preview_store().lock().unwrap().get_mut(preview_id) {
        entry.created_at = Instant::now() - IMPORT_PREVIEW_TTL;
    }
}

#[cfg(test)]
pub(super) fn pending_preview_count() -> usize {
    preview_store().lock().unwrap().len()
}

#[cfg(test)]
pub(super) fn clear_storage_import_previews_for_tests() {
    preview_store().lock().unwrap().clear();
}

#[cfg(test)]
pub(super) fn zeroize_json_value_for_tests(value: &mut Value) {
    zeroize_json_value(value);
}

#[derive(Debug)]
struct ParsedShareableStorage {
    record: StorageRecord,
    requested_exposure: bool,
    explicit_secrets: serde_json::Map<String, Value>,
}

fn split_internal_path(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(next) => current.push(next),
                None => current.push('\\'),
            },
            '.' => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
}

/// Normalize a required secret field to canonical RFC 6901, rejecting malformed
/// pointer escapes rather than silently producing a never-matching field.
fn canonical_secret_field(field: &str) -> McpResult<String> {
    match field.starts_with('/') {
        true => {
            let path = infimount_core::secrets::parse_secret_path(field).map_err(|_| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    "requiredSecretFields contains an invalid JSON Pointer",
                )
            })?;
            Ok(infimount_core::secrets::canonical_secret_path(&path))
        }
        false => Ok(infimount_core::secrets::canonicalize_secret_field(field)),
    }
}

pub(crate) fn secret_field_to_pointer(field: &str) -> String {
    let segments = if field.starts_with('/') {
        // Newer stores may already use JSON Pointer keys.
        return field.to_string();
    } else {
        split_internal_path(field)
    };
    format!(
        "/{}",
        segments
            .iter()
            .map(|segment| segment.replace('~', "~0").replace('/', "~1"))
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn registry_snapshot(storages: &[StorageRecord]) -> McpResult<Vec<u8>> {
    serde_json::to_vec(storages).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to fingerprint storage registry",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })
}

fn parse_shareable_json(input: &str) -> McpResult<Vec<ParsedShareableStorage>> {
    let value: Value = serde_json::from_str(input).map_err(|e| {
        sanitized_parse_error(
            McpErrorCode::ERR_INTERNAL,
            "failed to parse import JSON",
            "invalid_json",
            &e,
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
                        (map.contains_key("name") || map.contains_key("backend")).then_some(&value)
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
            ))
        }
    };

    items
        .into_iter()
        .map(|item| {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ImportedStorageWire {
                name: String,
                backend: String,
                #[serde(default)]
                config: Value,
                #[serde(
                    default,
                    rename = "requiredSecretFields",
                    alias = "required_secret_fields"
                )]
                required_secret_fields: Vec<String>,
                #[serde(default = "default_true")]
                enabled: bool,
                #[serde(default, rename = "mcpExposed", alias = "mcp_exposed")]
                mcp_exposed: bool,
                #[serde(default, rename = "readOnly", alias = "read_only")]
                read_only: bool,
                #[serde(default, rename = "mcpPolicy", alias = "mcp_policy")]
                mcp_policy: McpStoragePolicy,
            }
            let mut wire: ImportedStorageWire = serde_json::from_value(item).map_err(|e| {
                sanitized_parse_error(
                    McpErrorCode::ERR_INTERNAL,
                    "imported storage entry is invalid",
                    "invalid_entry",
                    &e,
                )
            })?;
            normalize_storage_policy(&mut wire.mcp_policy)?;

            let secret_names = infimount_core::secrets::discover_secret_field_names();
            let extracted =
                infimount_core::secrets::extract_secret_fields(&wire.config, &secret_names);
            infimount_core::secrets::strip_secret_fields(&mut wire.config, &secret_names);
            let explicit_secrets = extracted.iter().cloned().collect::<serde_json::Map<_, _>>();
            let mut required_secret_fields = wire
                .required_secret_fields
                .iter()
                .map(|field| canonical_secret_field(field))
                .collect::<McpResult<Vec<_>>>()?;
            required_secret_fields.extend(extracted.into_iter().map(|(field, _)| field));
            required_secret_fields.sort();
            required_secret_fields.dedup();

            let mut record = ImportedStorage {
                name: wire.name,
                backend: wire.backend,
                config: wire.config,
                enabled: wire.enabled,
                mcp_exposed: false,
                read_only: wire.read_only,
                id: None,
                created_at: None,
                updated_at: None,
            }
            .into_record()?;
            record.mcp_policy = wire.mcp_policy;
            record.secret_fields = required_secret_fields;
            Ok(ParsedShareableStorage {
                record,
                requested_exposure: wire.mcp_exposed,
                explicit_secrets,
            })
        })
        .collect()
}

fn preserve_existing_credentials(
    mut incoming: StorageRecord,
    existing: &StorageRecord,
) -> StorageRecord {
    incoming.id = existing.id.clone();
    incoming.created_at = existing.created_at.clone();
    incoming.updated_at = Utc::now().to_rfc3339();
    incoming.secret_ref = existing.secret_ref.clone();
    incoming.secret_fields.extend(
        existing
            .secret_fields
            .iter()
            .map(|field| infimount_core::secrets::canonicalize_secret_field(field)),
    );
    incoming.secret_fields.sort();
    incoming.secret_fields.dedup();
    incoming
}

fn change(record: &StorageRecord, change_type: impl Into<String>) -> StorageImportChange {
    StorageImportChange {
        name: record.name.clone(),
        backend: record.backend.clone(),
        change_type: change_type.into(),
    }
}

pub async fn preview_storage_import(
    ctx: &FsToolsContext,
    input: PreviewStorageImportInput,
) -> McpResult<StorageImportPreview> {
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
    let parsed = parse_shareable_json(&input.json)?;
    let existing = ctx.registry.load_all()?;
    let mut result = if input.mode == "replace" {
        Vec::new()
    } else {
        existing.clone()
    };
    let mut renames = Vec::new();
    let mut warnings = Vec::new();
    let mut missing_secret_fields = Vec::new();
    let mut secret_stages = Vec::new();
    let mut credential_replacement = false;

    for parsed_storage in parsed {
        let original_name = parsed_storage.record.name.clone();
        let existing_match = existing.iter().find(|record| record.name == original_name);
        if existing_match.is_some() && !parsed_storage.explicit_secrets.is_empty() {
            credential_replacement = true;
        }
        let mut incoming = parsed_storage.record;

        if input.mode == "replace" {
            if let Some(previous) = existing_match {
                incoming = preserve_existing_credentials(incoming, previous);
            }
            ensure_unique_name(&result, &incoming.name, None)?;
            result.push(incoming.clone());
        } else if let Some(index) = result
            .iter()
            .position(|record| record.name == incoming.name)
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
                    incoming = preserve_existing_credentials(incoming, &result[index]);
                    result[index] = incoming.clone();
                }
                "rename" => {
                    incoming.id = Uuid::new_v4().to_string();
                    incoming.name = next_renamed_name(&result, &incoming.name);
                    incoming.created_at = Utc::now().to_rfc3339();
                    incoming.updated_at = incoming.created_at.clone();
                    incoming.secret_ref = None;
                    ensure_unique_name(&result, &incoming.name, None)?;
                    renames.push(change(&incoming, format!("renamed from '{original_name}'")));
                    result.push(incoming.clone());
                }
                _ => unreachable!(),
            }
        } else {
            ensure_unique_name(&result, &incoming.name, None)?;
            result.push(incoming.clone());
        }

        let existing_bundle = match incoming.secret_ref.as_deref() {
            Some(account) => {
                match ctx.registry.secret_store().get_json(account) {
                    Ok(Some(Value::Object(bundle))) => bundle,
                    Ok(_) => {
                        warnings.push(format!(
                            "Stored credentials for '{}' are missing and must be re-entered.",
                            incoming.name
                        ));
                        serde_json::Map::new()
                    }
                    Err(_) => {
                        warnings.push(format!("Stored credentials for '{}' could not be verified and must be re-entered.", incoming.name));
                        serde_json::Map::new()
                    }
                }
            }
            None => serde_json::Map::new(),
        };
        let mut existing_bundle_value = Value::Object(existing_bundle);
        infimount_core::secrets::canonicalize_bundle_keys(&mut existing_bundle_value);
        let existing_bundle = existing_bundle_value
            .as_object()
            .cloned()
            .unwrap_or_default();
        let available = existing_bundle
            .keys()
            .chain(parsed_storage.explicit_secrets.keys())
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for field in &incoming.secret_fields {
            if !available.contains(field) {
                missing_secret_fields.push(MissingSecretField {
                    name: secret_field_to_pointer(field),
                    storage_name: incoming.name.clone(),
                });
            }
        }
        if !parsed_storage.explicit_secrets.is_empty() {
            secret_stages.push(SecretStage {
                record_id: incoming.id.clone(),
                explicit: parsed_storage.explicit_secrets,
            });
        }
        if parsed_storage.requested_exposure {
            warnings.push(format!(
                "Storage '{}' requested MCP exposure; shareable imports always disable exposure.",
                incoming.name
            ));
        }
    }

    assign_import_revisions(&existing, &mut result)?;
    let mut additions = Vec::new();
    let mut updates = Vec::new();
    let mut removals = Vec::new();
    let mut policy_changes = Vec::new();
    let mut exposure_changes = Vec::new();

    for planned in &result {
        if let Some(previous) = existing.iter().find(|record| record.id == planned.id) {
            if planned.backend != previous.backend
                || planned.config != previous.config
                || planned.enabled != previous.enabled
                || planned.read_only != previous.read_only
            {
                updates.push(change(planned, "storage configuration changed"));
            }
            if planned.mcp_policy != previous.mcp_policy {
                policy_changes.push(change(planned, "MCP policy changed"));
            }
            if planned.mcp_exposed != previous.mcp_exposed {
                exposure_changes.push(change(
                    planned,
                    if planned.mcp_exposed {
                        "MCP exposure enabled"
                    } else {
                        "MCP exposure disabled"
                    },
                ));
            }
        } else if !renames.iter().any(|renamed| renamed.name == planned.name) {
            additions.push(change(planned, "new storage"));
        }
    }
    if input.mode == "replace" {
        for previous in &existing {
            if !result.iter().any(|planned| planned.id == previous.id) {
                removals.push(change(previous, "storage removed"));
            }
        }
    }

    missing_secret_fields
        .sort_by(|a, b| (&a.storage_name, &a.name).cmp(&(&b.storage_name, &b.name)));
    missing_secret_fields.dedup_by(|a, b| a.storage_name == b.storage_name && a.name == b.name);

    let mut confirmation_reasons = Vec::new();
    if input.mode == "replace" {
        confirmation_reasons.push("replace mode replaces all configured storages".to_string());
    }
    if !updates.is_empty() {
        confirmation_reasons.push("existing storages will be updated".to_string());
    }
    if !removals.is_empty() {
        confirmation_reasons.push("storages will be removed".to_string());
    }
    if !policy_changes.is_empty() {
        confirmation_reasons.push("MCP access policies will change".to_string());
    }
    if !exposure_changes.is_empty() {
        confirmation_reasons.push("MCP exposure settings will change".to_string());
    }
    if credential_replacement {
        confirmation_reasons.push("existing credentials will be replaced".to_string());
    }
    let requires_confirmation = !confirmation_reasons.is_empty();

    let preview_id = Uuid::new_v4().to_string();
    let preview = StorageImportPreview {
        preview_id: preview_id.clone(),
        mode: input.mode,
        on_conflict: input.on_conflict,
        additions,
        updates,
        renames,
        removals,
        policy_changes,
        exposure_changes,
        missing_secret_fields,
        warnings,
        requires_confirmation,
        confirmation_reasons,
    };
    let entry = PreviewEntry {
        changes: preview.clone(),
        storages: result,
        secret_stages,
        base_registry_snapshot: registry_snapshot(&existing)?,
        created_at: Instant::now(),
    };
    ensure_preview_expiry_task();
    insert_preview_entry(preview_id, entry);
    Ok(preview)
}

fn build_import_journal(
    registry: &crate::registry::StorageRegistry,
    original_present: bool,
    original_bytes: &[u8],
    replacement_records: &[StorageRecord],
    staged_secret_accounts: Vec<String>,
    obsolete_secret_accounts: Vec<String>,
) -> McpResult<(std::path::PathBuf, ImportTransactionJournal)> {
    let replacement_bytes = registry.serialize_records(replacement_records)?;
    let path = registry.import_journal_path(&Uuid::new_v4().to_string())?;
    let journal = ImportTransactionJournal {
        version: IMPORT_JOURNAL_VERSION,
        state: ImportTransactionState::Prepared,
        original_present,
        original_registry_base64: base64::engine::general_purpose::STANDARD.encode(original_bytes),
        replacement_registry_base64: base64::engine::general_purpose::STANDARD
            .encode(&replacement_bytes),
        staged_secret_accounts,
        obsolete_secret_accounts,
    };
    registry.write_import_journal(&path, &journal)?;
    Ok((path, journal))
}

fn remove_pending_backup(path: &std::path::Path) -> McpResult<()> {
    std::fs::remove_file(path).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to remove completed import rollback journal",
        )
    })
}

fn rollback_registry_if_matches(
    ctx: &FsToolsContext,
    imported: &[StorageRecord],
    original: &[StorageRecord],
) -> McpResult<bool> {
    ctx.registry
        .restore_all_if_matches(imported, original)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "import failed and storage registry rollback failed",
            )
        })
}

fn abort_import_before_commit(
    ctx: &FsToolsContext,
    journal_path: &std::path::Path,
    rollback: &mut Vec<(String, Option<Value>)>,
    error: crate::errors::McpError,
) -> crate::errors::McpError {
    let rollback_error = rollback_import_secrets(
        ctx.registry.secret_store().as_ref(),
        std::mem::take(rollback),
    )
    .err();
    if rollback_error.is_none() {
        let _ = remove_pending_backup(journal_path);
    }
    err_with_details(
        error.code,
        error.message,
        serde_json::json!({ "secretRollbackError": rollback_error.map(|error| error.message) }),
    )
}

pub async fn apply_storage_import(
    ctx: &FsToolsContext,
    input: ApplyStorageImportInput,
) -> McpResult<ApplyStorageImportResult> {
    apply_storage_import_with_validator(ctx, input, |_| Ok(())).await
}

pub async fn apply_storage_import_with_validator<F>(
    ctx: &FsToolsContext,
    input: ApplyStorageImportInput,
    validate_result: F,
) -> McpResult<ApplyStorageImportResult>
where
    F: FnOnce(&[StorageRecord]) -> McpResult<()>,
{
    let _transaction = ctx.registry.acquire_configuration_transaction()?;
    apply_storage_import_with_validator_locked(ctx, input, validate_result).await
}

pub async fn apply_storage_import_with_validator_locked<F>(
    ctx: &FsToolsContext,
    input: ApplyStorageImportInput,
    validate_result: F,
) -> McpResult<ApplyStorageImportResult>
where
    F: FnOnce(&[StorageRecord]) -> McpResult<()>,
{
    let entry = {
        let store = preview_store().lock().unwrap();
        match store.get(&input.preview_id) {
            Some(entry) if entry.created_at.elapsed() < IMPORT_PREVIEW_TTL => entry.clone(),
            _ => {
                drop(store);
                remove_preview_entry_zeroized(&input.preview_id);
                return Err(err(
                    McpErrorCode::ERR_IMPORT_PREVIEW_EXPIRED,
                    "import preview not found or expired; re-preview the import",
                ));
            }
        }
    };
    if entry.changes.requires_confirmation && !input.confirmed {
        return Err(err_with_details(
            McpErrorCode::ERR_IMPORT_CONFIRMATION_REQUIRED,
            "this import requires explicit confirmation",
            serde_json::json!({ "confirmationReasons": entry.changes.confirmation_reasons }),
        ));
    }
    if !entry.changes.missing_secret_fields.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_SECRET_NOT_FOUND,
            "required credentials are missing; add them to the import JSON and preview again",
            serde_json::json!({ "missingSecretFields": entry.changes.missing_secret_fields }),
        ));
    }

    let existing = ctx.registry.load_all()?;
    if registry_snapshot(&existing)? != entry.base_registry_snapshot {
        return Err(err(
            McpErrorCode::ERR_IMPORT_PREVIEW_STALE,
            "storage registry has changed since preview; re-preview the import",
        ));
    }

    let mut merged = entry.storages.clone();
    // Desktop control-plane callers can enforce additional persisted-state
    // invariants (for example workspace-to-storage references) before any
    // registry, rollback-journal, or native-secret mutation occurs.
    validate_result(&merged)?;

    // ---- Prepare the whole transaction in memory before touching keyring. ----
    // Preallocate every staged account, merge explicit secrets into the future
    // bundles, and record which references are being retired.
    let mut staged_bundles: Vec<(String, Value)> = Vec::new();
    for stage in &entry.secret_stages {
        let Some(record) = merged
            .iter_mut()
            .find(|record| record.id == stage.record_id)
        else {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "import secret stage did not match its storage",
            ));
        };
        let mut bundle = match record.secret_ref.as_deref() {
            Some(account) => match ctx.registry.secret_store().get_json(account) {
                Ok(bundle) => bundle.unwrap_or_else(|| Value::Object(serde_json::Map::new())),
                Err(_) => {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to read existing credentials",
                    ));
                }
            },
            None => Value::Object(serde_json::Map::new()),
        };
        infimount_core::secrets::canonicalize_bundle_keys(&mut bundle);
        let Some(object) = bundle.as_object_mut() else {
            return Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "stored secret bundle is invalid",
            ));
        };
        object.extend(stage.explicit.clone());
        let mut secret_fields = object.keys().cloned().collect::<Vec<_>>();
        secret_fields.sort();
        let account = format!("storage/{}/import/{}", record.id, Uuid::new_v4());
        staged_bundles.push((account.clone(), bundle));
        record.secret_ref = Some(account);
        record.secret_fields = secret_fields;
    }
    assign_import_revisions(&existing, &mut merged)?;
    let retained_refs = merged
        .iter()
        .filter_map(|record| record.secret_ref.as_deref())
        .collect::<std::collections::HashSet<_>>();
    let obsolete_accounts = existing
        .iter()
        .filter_map(|record| record.secret_ref.clone())
        .filter(|account| !retained_refs.contains(account.as_str()))
        .collect::<Vec<_>>();
    let (original_present, original_bytes) = ctx.registry.registry_bytes()?;
    let staged_accounts = staged_bundles
        .iter()
        .map(|(account, _)| account.clone())
        .collect::<Vec<_>>();

    // Persist a complete durable journal before changing either the registry or
    // the native secret store. All account IDs are present up front.
    let (pending_backup, journal) = build_import_journal(
        &ctx.registry,
        original_present,
        &original_bytes,
        &merged,
        staged_accounts,
        obsolete_accounts,
    )?;
    let mut rollback = Vec::new();
    for (account, bundle) in &staged_bundles {
        if ctx
            .registry
            .secret_store()
            .put_json(account, bundle)
            .is_err()
        {
            return Err(abort_import_before_commit(
                ctx,
                &pending_backup,
                &mut rollback,
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage imported credentials",
                ),
            ));
        }
        rollback.push((account.clone(), None));
    }
    let mut journal = journal;
    journal.state = ImportTransactionState::SecretsStaged;
    if let Err(error) = ctx.registry.write_import_journal(&pending_backup, &journal) {
        return Err(abort_import_before_commit(
            ctx,
            &pending_backup,
            &mut rollback,
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to advance import journal state: {error}"),
                serde_json::json!({}),
            ),
        ));
    }
    if let Err(error) = ctx.registry.replace_all_atomic_verified(&existing, &merged) {
        let rollback_error = rollback_import_secrets(
            ctx.registry.secret_store().as_ref(),
            std::mem::take(&mut rollback),
        )
        .err();
        let registry_error = match rollback_registry_if_matches(ctx, &merged, &existing) {
            Ok(true) => None,
            Ok(false) => {
                Some("registry rollback refused because persisted state advanced".to_string())
            }
            Err(error) => Some(error.message),
        };
        // Keep the durable journal when either rollback stage is incomplete;
        // startup recovery will resolve it instead of treating CAS refusal as success.
        if rollback_error.is_some() || registry_error.is_some() {
            return Err(err_with_details(
                error.code,
                error.message.clone(),
                serde_json::json!({
                    "secretRollbackError": rollback_error.map(|e| e.message),
                    "registryRollbackError": registry_error,
                }),
            ));
        }
        let _ = remove_pending_backup(&pending_backup);
        return Err(error);
    }

    // ---- Commit-marker handling (§4.7). The registry is now committed. ----
    let cleanup_accounts = journal.obsolete_secret_accounts.clone();
    let mut warnings = entry.changes.warnings.clone();
    journal.state = ImportTransactionState::Committed;
    let committed_marker_failed = ctx
        .registry
        .write_import_journal(&pending_backup, &journal)
        .is_err();
    if committed_marker_failed {
        warnings.push(
            "Import committed; the transaction journal could not be marked committed and will be resolved during cleanup.".to_string(),
        );
    }

    let cleanup_result = ctx.registry.with_locked_mutation(|current| {
        let active = current
            .iter()
            .filter_map(|record| record.secret_ref.as_deref())
            .collect::<std::collections::HashSet<_>>();
        let mut failed = Vec::new();
        for account in &cleanup_accounts {
            if !active.contains(account.as_str())
                && ctx.registry.secret_store().delete(account).is_err()
            {
                failed.push(account.clone());
            }
        }
        Ok(failed)
    });
    let mut cleanup_durable = true;
    match cleanup_result {
        Ok(failed) => {
            for account in &failed {
                if append_cleanup_journal_at(ctx.registry.path(), account).is_err() {
                    cleanup_durable = false;
                }
            }
            // Emit "cleanup pending" only when an account actually failed
            // deletion, not merely because a cleanup pass was attempted.
            if !failed.is_empty() {
                warnings.push("Credential cleanup is pending and will be retried.".to_string());
            }
        }
        Err(error) => {
            cleanup_durable = false;
            warnings.push(format!(
                "The completed import transaction remains journaled for recovery: {}",
                error.message
            ));
        }
    }
    // The import is committed. The durable journal may be removed only when
    // every obsolete account was either deleted or durably journaled to the
    // strict cleanup file. Removing it otherwise would lose the only durable
    // list of the remaining cleanup obligation.
    if cleanup_durable {
        if remove_pending_backup(&pending_backup).is_err() {
            let _ = ctx.registry.mark_configuration_blocked();
            warnings.push(
                "Import committed, but configuration mutations are blocked until the pending import journal is recovered.".to_string(),
            );
        }
    } else {
        let _ = ctx.registry.mark_configuration_blocked();
        warnings.push(
            "Import committed, but configuration mutations are blocked until the pending import journal is recovered.".to_string(),
        );
    }
    remove_preview_entry_zeroized(&input.preview_id);
    Ok(ApplyStorageImportResult {
        applied: entry.storages.len(),
        warnings,
    })
}
