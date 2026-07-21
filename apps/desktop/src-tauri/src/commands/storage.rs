#![allow(non_snake_case)]

use chrono::Utc;
use fs2::FileExt;
use infimount_core::{operations, schema::StorageKindSchema, secrets, CoreError, Entry};
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::opendal_adapter::{get_capabilities, StorageBackendCapabilities};
use infimount_mcp::policy::{migrate_legacy_policy, normalize_storage_policy, McpStoragePolicy};
use infimount_mcp::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use infimount_mcp::tools_storage::{
    export_config, import_config, validate_storage_record, ExportConfigInput, ExportConfigOutput,
    ImportConfigInput, ImportConfigOutput, ValidateStorageOutput,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::time::Duration;
use tauri::State;

use crate::state::{AppState, PendingOAuthClaim, PendingOAuthSession, SecretMutation};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDraft {
    #[serde(default)]
    pub storage_id: Option<String>,
    pub name: String,
    pub backend: String,
    pub config: Value,
    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
    #[serde(default)]
    pub secret_mutations: HashMap<String, SecretMutation>,
    #[serde(default)]
    pub oauth_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStoragesRequest {
    pub json: String,
    pub mode: String,
    pub on_conflict: String,
}

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries(&op, &path).await
}

#[tauri::command]
pub async fn list_entries_recursive(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries_recursive(&op, &path).await
}

#[tauri::command]
pub async fn stat_entry(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Entry, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::stat_entry(&op, &path).await
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::read_full(&op, &path).await
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    data: Vec<u8>,
    userMetadata: Option<HashMap<String, String>>,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::write_full_with_user_metadata(&op, &path, &data, userMetadata).await
}

#[tauri::command]
pub async fn create_directory(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::create_directory(&op, &path).await
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete(&op, &path).await
}

#[tauri::command]
pub async fn upload_dropped_files(
    state: State<'_, AppState>,
    sourceId: String,
    paths: Vec<String>,
    targetDir: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::upload_files_from_paths(&op, paths, targetDir).await
}

#[tauri::command]
pub fn list_storages(state: State<'_, AppState>) -> Result<Vec<StorageRecord>, McpError> {
    state.list_storages()
}

fn claim_oauth_session(state: &State<'_, AppState>, id: &str) -> McpResult<PendingOAuthSession> {
    match state.pending_oauth.claim(id) {
        PendingOAuthClaim::Session(session) => Ok(session),
        PendingOAuthClaim::Expired => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_EXPIRED,
            "OAuth session expired",
        )),
        PendingOAuthClaim::AlreadyUsed => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_ALREADY_USED,
            "OAuth session was already used",
        )),
        PendingOAuthClaim::InUse => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_IN_USE,
            "OAuth session is already being saved",
        )),
        PendingOAuthClaim::NotFound => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
            "OAuth session was not found",
        )),
    }
}

#[tauri::command]
pub fn add_storage(
    state: State<'_, AppState>,
    storage: StorageDraft,
) -> Result<StorageRecord, McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let schema_secret_names = secrets::discover_secret_field_names();

    // Handle OAuth session if present
    if let Some(oauth_id) = storage.oauth_session_id.clone() {
        let session = claim_oauth_session(&state, &oauth_id)?;
        let expected_backend = if session.provider == "gdrive" {
            "gdrive"
        } else {
            "onedrive"
        };
        if storage.backend != expected_backend {
            state.pending_oauth.restore(session);
            return Err(err(
                McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
                "OAuth provider does not match storage backend",
            ));
        }
        let public = secrets::merge_secret_config(&storage.config, &session.public_config);
        let merged = secrets::merge_secret_config(&public, &session.secret_config);
        return match add_storage_with_config(&state, name, storage, merged, &schema_secret_names) {
            Ok(result) => {
                state.pending_oauth.complete(session);
                Ok(result)
            }
            Err(error) => {
                state.pending_oauth.restore(session);
                Err(error)
            }
        };
    }

    let config = storage.config.clone();
    add_storage_with_config(&state, name, storage, config, &schema_secret_names)
}

fn add_storage_with_config(
    state: &State<'_, AppState>,
    name: String,
    storage: StorageDraft,
    config: Value,
    schema_secret_names: &[String],
) -> Result<StorageRecord, McpError> {
    let mut record = StorageRecord::new(name.clone(), storage.backend.clone(), config);
    record.enabled = storage.enabled;
    record.mcp_exposed = storage.mcp_exposed;
    record.read_only = storage.read_only;
    let extracted = secrets::extract_secret_fields(&record.config, schema_secret_names);
    secrets::strip_secret_fields(&mut record.config, schema_secret_names);
    let mut bundle = Value::Object(extracted.iter().cloned().collect());
    apply_secret_mutations_to_bundle(&mut bundle, &storage.secret_mutations)?;
    let account = format!("storage/{}", record.id);
    let has_secrets = bundle.as_object().is_some_and(|object| !object.is_empty());
    if has_secrets {
        if state.secret_store.put_json(&account, &bundle).is_err() {
            if state.secret_store.delete(&account).is_err() {
                let journal_path =
                    infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
                append_secret_cleanup(&journal_path, &account).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "credential rollback failed and could not be journaled",
                    )
                })?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "credential write failed; cleanup is pending",
                ));
            }
            return Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to store credentials",
            ));
        }
        record.secret_ref = Some(account.clone());
        record.secret_fields = bundle.as_object().unwrap().keys().cloned().collect();
    }
    let result = state.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &name, None)?;
        storages.push(record.clone());
        Ok(record.clone())
    });
    if result.is_err() && has_secrets && state.secret_store.delete(&account).is_err() {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        append_secret_cleanup(&journal_path, &account)?;
    }
    result
}

fn apply_secret_mutations_to_bundle(
    bundle: &mut Value,
    mutations: &HashMap<String, SecretMutation>,
) -> McpResult<()> {
    let object = bundle.as_object_mut().ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
            "stored secret bundle is invalid",
        )
    })?;
    for (field, mutation) in mutations {
        match mutation {
            SecretMutation::Set { value } => {
                let value = value.trim();
                if value.is_empty() || value == "********" {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_PATH,
                        "secret value must not be empty or masked",
                    ));
                }
                object.insert(field.clone(), Value::String(value.to_string()));
            }
            SecretMutation::Clear => {
                object.remove(field);
            }
            SecretMutation::Keep => {}
        }
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStorageResult {
    pub storage: StorageRecord,
    pub warning: Option<String>,
}

#[tauri::command]
pub fn update_storage(
    state: State<'_, AppState>,
    storageId: String,
    mut storage: StorageDraft,
) -> Result<UpdateStorageResult, McpError> {
    let claimed = if let Some(oauth_id) = storage.oauth_session_id.as_deref() {
        let session = claim_oauth_session(&state, oauth_id)?;
        let expected_backend = if session.provider == "gdrive" {
            "gdrive"
        } else {
            "onedrive"
        };
        if storage.backend != expected_backend {
            state.pending_oauth.restore(session);
            return Err(err(
                McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
                "OAuth provider does not match storage backend",
            ));
        }
        let public = secrets::merge_secret_config(&storage.config, &session.public_config);
        storage.config = secrets::merge_secret_config(&public, &session.secret_config);
        if session.secret_config.get("refreshToken").is_some() {
            storage
                .secret_mutations
                .insert("accessToken".to_string(), SecretMutation::Clear);
        }
        if session.secret_config.get("accessToken").is_some() {
            storage
                .secret_mutations
                .insert("refreshToken".to_string(), SecretMutation::Clear);
        }
        Some(session)
    } else {
        None
    };

    let result = update_storage_with_draft(&state, storageId, storage);
    if let Some(session) = claimed {
        if result.is_ok() {
            state.pending_oauth.complete(session);
        } else {
            state.pending_oauth.restore(session);
        }
    }
    result.map(|(storage, warning)| UpdateStorageResult { storage, warning })
}

fn update_storage_with_draft(
    state: &State<'_, AppState>,
    storageId: String,
    storage: StorageDraft,
) -> Result<(StorageRecord, Option<String>), McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let secret_store = state.secret_store.clone();
    let schema_secret_names = secrets::discover_secret_field_names();
    let mut previous_account = String::new();
    let mut staged_account = String::new();
    let mut previous_bundle: Option<Value> = None;
    let mut staged_secret = false;
    let result = state.registry.with_locked_mutation(|storages| {
        let idx = storages
            .iter()
            .position(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    "storage not found",
                    json!({ "storage_id": storageId }),
                )
            })?;
        ensure_unique_name(storages, &name, Some(storageId.as_str()))?;

        previous_account = storages[idx]
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{storageId}"));
        staged_account = format!(
            "storage/{storageId}/revision/{}/{}",
            storages[idx].revision.saturating_add(1),
            uuid::Uuid::new_v4()
        );
        previous_bundle = secret_store.get_json(&previous_account).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to access stored credentials",
            )
        })?;
        if previous_bundle.is_none()
            && (storages[idx].secret_ref.is_some() || !storages[idx].secret_fields.is_empty())
        {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        let mut staged_bundle = previous_bundle.clone().unwrap_or_else(|| json!({}));
        let extracted = secrets::extract_secret_fields(&storage.config, &schema_secret_names);
        if let Some(object) = staged_bundle.as_object_mut() {
            object.extend(extracted);
        }
        apply_secret_mutations_to_bundle(&mut staged_bundle, &storage.secret_mutations)?;
        let has_secrets = staged_bundle
            .as_object()
            .is_some_and(|object| !object.is_empty());
        if has_secrets {
            if secret_store
                .put_json(&staged_account, &staged_bundle)
                .is_err()
            {
                if secret_store.delete(&staged_account).is_err() {
                    let journal_path =
                        infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
                    append_secret_cleanup(&journal_path, &staged_account)?;
                }
                return Err(err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage updated credentials",
                ));
            }
            staged_secret = true;
        }

        let mut updated = storages[idx].clone();
        updated.name = name.clone();
        updated.backend = storage.backend.clone();
        updated.enabled = storage.enabled;
        updated.mcp_exposed = storage.mcp_exposed;
        updated.read_only = storage.read_only;
        updated.config = storage.config.clone();
        secrets::strip_secret_fields(&mut updated.config, &schema_secret_names);
        updated.secret_ref = has_secrets.then(|| staged_account.clone());
        updated.secret_fields = staged_bundle
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        updated.revision = updated.revision.saturating_add(1);
        updated.updated_at = Utc::now().to_rfc3339();
        storages[idx] = updated.clone();
        Ok(updated)
    });
    if result.is_err() && staged_secret && secret_store.delete(&staged_account).is_err() {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        append_secret_cleanup(&journal_path, &staged_account)?;
    }

    let mut warning = None;
    if result.is_ok()
        && previous_bundle.is_some()
        && previous_account != staged_account
        && secret_store.delete(&previous_account).is_err()
    {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        warning = Some(
            if append_secret_cleanup(&journal_path, &previous_account).is_ok() {
                "Previous credential cleanup is pending and will be retried."
            } else {
                "Previous credential cleanup failed and could not be journaled; remove the old native secret-store entry manually."
            }
            .to_string(),
        );
    }
    result.map(|storage| (storage, warning))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveStorageResult {
    pub removed: bool,
    pub warning: Option<String>,
}

#[tauri::command]
pub fn remove_storage(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<RemoveStorageResult, McpError> {
    let secret_store = state.secret_store.clone();
    let mut secret_ref_to_delete: Option<String> = None;

    state.registry.with_locked_mutation(|storages| {
        let original_len = storages.len();
        let target = storages.iter().find(|s| s.id == storageId);
        secret_ref_to_delete = target.and_then(|s| s.secret_ref.clone());
        storages.retain(|storage| storage.id != storageId);
        if storages.len() == original_len {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("storage '{}' not found", storageId),
                json!({ "storage_id": storageId }),
            ));
        }
        Ok(())
    })?;

    // Delete keyring entry after successful registry mutation
    if let Some(ref secret_ref) = secret_ref_to_delete {
        if secret_store.delete(secret_ref).is_err() {
            let journal_path =
                infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
            let warning = if append_secret_cleanup(&journal_path, secret_ref).is_ok() {
                "Credential cleanup is pending and will be retried.".to_string()
            } else {
                "Credential cleanup failed and could not be journaled; remove the native secret-store entry manually.".to_string()
            };
            return Ok(RemoveStorageResult {
                removed: true,
                warning: Some(warning),
            });
        }
    }

    Ok(RemoveStorageResult {
        removed: true,
        warning: None,
    })
}

fn append_secret_cleanup(path: &std::path::Path, account: &str) -> McpResult<()> {
    let lock_path = path.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        infimount_core::atomic_file::create_dir_all(parent)
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
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
    let mut document = if path.exists() {
        serde_json::from_slice::<Value>(
            &std::fs::read(path)
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to read cleanup journal"))?,
        )
        .unwrap_or_else(|_| json!({ "pending": [] }))
    } else {
        json!({ "pending": [] })
    };
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
        pending.push(json!({ "account": account, "createdAt": Utc::now().to_rfc3339() }));
    }
    let payload = serde_json::to_vec_pretty(&document).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to create cleanup journal",
        )
    })?;
    infimount_core::atomic_file::atomic_write_file(
        path,
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

#[tauri::command]
pub fn update_mcp_storage_policy(
    state: State<'_, AppState>,
    storageId: String,
    policy: McpStoragePolicy,
) -> Result<StorageRecord, McpError> {
    state.registry.with_locked_mutation(|storages| {
        let storage = storages
            .iter_mut()
            .find(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{}' not found", storageId),
                    serde_json::json!({ "storage_id": storageId }),
                )
            })?;

        storage.mcp_policy = normalize_mcp_policy(policy)?;
        storage.updated_at = Utc::now().to_rfc3339();
        Ok(storage.clone())
    })
}

fn normalize_mcp_policy(mut policy: McpStoragePolicy) -> McpResult<McpStoragePolicy> {
    migrate_legacy_policy(&mut policy)?;
    normalize_storage_policy(&mut policy)?;
    Ok(policy)
}

#[tauri::command]
pub async fn verify_storage(
    state: State<'_, AppState>,
    storage: StorageDraft,
) -> Result<ValidateStorageOutput, McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let mut config = storage.config.clone();
    let mut bundle = if let Some(storage_id) = storage.storage_id.as_deref() {
        let record = state.find_storage_by_id(storage_id)?;
        if record.backend != storage.backend {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "storage verification reference does not match the selected backend",
            ));
        }
        let account = record
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{}", record.id));
        let stored = state.secret_store.get_json(&account).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to access stored credentials",
            )
        })?;
        if stored.is_none() && (record.secret_ref.is_some() || !record.secret_fields.is_empty()) {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        stored.unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    apply_secret_mutations_to_bundle(&mut bundle, &storage.secret_mutations)?;
    config = secrets::merge_secret_config(&config, &bundle);
    if let Some(session_id) = storage.oauth_session_id.as_deref() {
        let (provider, public_config, secret_config) =
            state.pending_oauth.snapshot(session_id).ok_or_else(|| {
                err(
                    McpErrorCode::ERR_OAUTH_SESSION_EXPIRED,
                    "OAuth session is missing or expired",
                )
            })?;
        if provider != storage.backend {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "OAuth session does not match the selected storage backend",
            ));
        }
        config = secrets::merge_secret_config(&config, &public_config);
        config = secrets::merge_secret_config(&config, &secret_config);
    }
    let mut record = StorageRecord::new(name, storage.backend, config);
    record.enabled = storage.enabled;
    record.mcp_exposed = storage.mcp_exposed;
    record.read_only = storage.read_only;
    validate_storage_record(&record).await
}

#[tauri::command]
pub async fn import_storage_config(
    state: State<'_, AppState>,
    request: ImportStoragesRequest,
) -> Result<ImportConfigOutput, McpError> {
    import_config(
        &state.fs_context()?,
        ImportConfigInput {
            json: request.json,
            mode: request.mode,
            on_conflict: request.on_conflict,
        },
    )
    .await
}

#[tauri::command]
pub async fn export_storage_config(
    state: State<'_, AppState>,
    includeSecrets: bool,
) -> Result<ExportConfigOutput, McpError> {
    export_config(
        &state.fs_context()?,
        ExportConfigInput {
            include_secrets: includeSecrets,
        },
    )
    .await
}

#[tauri::command]
pub fn list_storage_schemas() -> Result<Vec<StorageKindSchema>, CoreError> {
    infimount_core::schema::list_storage_schemas()
}

#[tauri::command]
pub fn get_storage_capabilities(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<StorageBackendCapabilities, CoreError> {
    let op = state.operator_for_storage_id(&storageId)?;
    Ok(get_capabilities(&op))
}

#[tauri::command]
pub async fn generate_download_link(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    expiresSeconds: u64,
) -> Result<String, CoreError> {
    if !(60..=86_400).contains(&expiresSeconds) {
        return Err(CoreError::Config(
            "expiresSeconds must be between 60 and 86400".to_string(),
        ));
    }

    let op = state.operator_for_storage_id(&sourceId)?;
    let caps = op.info().full_capability();
    if !caps.presign_read {
        return Err(CoreError::Config(
            "storage backend does not support presigned download links".to_string(),
        ));
    }

    let metadata = op.stat(&path).await?;
    if metadata.is_dir() {
        return Err(CoreError::Config(
            "download links can only be created for files".to_string(),
        ));
    }

    let presigned = op
        .presign_read(&path, Duration::from_secs(expiresSeconds))
        .await?;
    Ok(presigned.uri().to_string())
}

#[tauri::command]
pub async fn list_versions(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    let result =
        operations::list_file_versions(&op, &path, limit.unwrap_or(100), cursor.as_deref()).await?;
    Ok(serde_json::to_value(result).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn read_file_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::read_file_version(&op, &path, &version).await
}

#[tauri::command]
pub async fn delete_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete_file_version(&op, &path, &version).await?;
    Ok(serde_json::json!({ "deleted": true, "path": path, "version": version }))
}

fn validate_storage_draft(storage: &StorageDraft) -> McpResult<()> {
    if !storage.config.is_object() {
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "storage config must be a JSON object",
            serde_json::json!({}),
        ));
    }

    if !matches!(
        storage.backend.as_str(),
        "local"
            | "s3"
            | "b2"
            | "oss"
            | "cos"
            | "obs"
            | "azure_blob"
            | "webdav"
            | "gcs"
            | "gdrive"
            | "onedrive"
            | "sftp"
            | "ftp"
    ) {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{}'", storage.backend),
            serde_json::json!({ "backend": storage.backend }),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_storage_draft_accepts_all_desktop_backends() {
        for backend in [
            "local",
            "s3",
            "b2",
            "oss",
            "cos",
            "obs",
            "azure_blob",
            "webdav",
            "gcs",
            "gdrive",
            "onedrive",
            "sftp",
            "ftp",
        ] {
            let storage = StorageDraft {
                storage_id: None,
                name: format!("{backend} storage"),
                backend: backend.to_string(),
                config: serde_json::json!({}),
                enabled: true,
                mcp_exposed: false,
                read_only: false,
                oauth_session_id: None,
                secret_mutations: HashMap::new(),
            };
            validate_storage_draft(&storage).expect("backend should be accepted");
        }
    }

    #[test]
    fn normalize_mcp_policy_uses_shared_v2_normalization() {
        let policy = McpStoragePolicy {
            denied_paths: vec!["shared/tmp/../public".to_string()],
            ..McpStoragePolicy::default()
        };
        let normalized = normalize_mcp_policy(policy).expect("normalize policy");
        assert_eq!(normalized.denied_paths, vec!["shared/public"]);
    }
}
