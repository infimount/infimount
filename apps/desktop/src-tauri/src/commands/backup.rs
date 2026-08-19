use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use infimount_core::backup::{self, BackupPayload};
use infimount_core::workspaces::{
    validate_workspace_metadata, WorkspaceRecord, WORKSPACE_RECORD_SCHEMA_VERSION,
};
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode};
use infimount_mcp::policy::{
    normalize_policy_path, normalize_storage_policy, McpAccessMode, McpRuleSource,
    MCP_POLICY_VERSION,
};
use infimount_mcp::registry::{StorageRecord, STORAGE_RECORD_SCHEMA_VERSION};
use infimount_mcp::settings::McpSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::app_settings::AppSettings;
use crate::state::AppState;

const RESTORE_PREVIEW_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PENDING_RESTORE_PREVIEWS: usize = 8;
const RESTORE_JOURNAL_VERSION: u32 = 1;
const RESTORE_JOURNAL_KEY_ACCOUNT: &str = "recovery/restore-transaction";

struct SensitiveString(String);

impl SensitiveString {
    fn new(value: String) -> Self {
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for SensitiveString {
    fn drop(&mut self) {
        backup::zeroize(&mut self.0);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupInput {
    pub passphrase: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupOutput {
    pub armored: String,
    pub storage_count: usize,
    pub has_native_secrets: bool,
}

#[tauri::command]
pub async fn create_recovery_backup(
    state: State<'_, AppState>,
    request: CreateBackupInput,
) -> Result<CreateBackupOutput, McpError> {
    let _transaction_guard = state.lifecycle_mutation.lock().await;
    let passphrase = SensitiveString::new(request.passphrase);
    if passphrase.as_str().len() < 8 {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "backup passphrase must contain at least 8 characters",
        ));
    }

    let storages = state.registry.load_all().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to load storage registry for backup",
        )
    })?;
    let mcp_settings = state.settings_store.load().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to load MCP settings for backup",
        )
    })?;
    let app_settings = state.app_settings_store.load().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to load app settings for backup",
        )
    })?;
    let workspaces = state.workspaces.load_all().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to load workspace registry for backup",
        )
    })?;

    let portable = make_portable_backup_state(&state, &storages, &mcp_settings)?;
    let storage_values = portable
        .storages
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize storage registry",
            )
        })?;
    let mcp_value = serde_json::to_value(&portable.mcp_settings).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize MCP settings",
        )
    })?;
    let app_value = serde_json::to_value(&app_settings).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize app settings",
        )
    })?;
    let workspace_value = serde_json::to_value(&workspaces)
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to serialize workspaces"))?;

    let has_native_secrets = !portable.secrets.is_empty();
    let payload = BackupPayload::new(
        storage_values,
        Some(mcp_value),
        Some(app_value),
        Some(workspace_value),
        portable.secrets,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to create backup payload",
        )
    })?;

    let armored = backup::encrypt_backup(passphrase.as_str(), &payload)
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to encrypt backup"))?;
    // Fail closed if this exact build cannot reopen and validate what it produced.
    // This catches format/checksum regressions before users trust the returned backup.
    let verified = backup::decrypt_backup(passphrase.as_str(), &armored).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "created backup failed immediate verification",
        )
    })?;
    if !verified.verify() {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "created backup failed immediate verification",
        ));
    }

    Ok(CreateBackupOutput {
        armored,
        storage_count: storages.len(),
        has_native_secrets,
    })
}

struct PortableBackupState {
    storages: Vec<StorageRecord>,
    mcp_settings: McpSettings,
    secrets: HashMap<String, String>,
}

fn make_portable_backup_state(
    state: &AppState,
    storages: &[StorageRecord],
    mcp_settings: &McpSettings,
) -> Result<PortableBackupState, McpError> {
    let mut portable_storages = storages.to_vec();
    let mut portable_settings = mcp_settings.clone();
    let mut source_to_portable = HashMap::<String, String>::new();
    let mut secrets = HashMap::new();

    for storage in &mut portable_storages {
        let Some(source_account) = storage.secret_ref.clone() else {
            continue;
        };
        let portable_account = source_to_portable
            .entry(source_account.clone())
            .or_insert_with(|| format!("backup/storage/{}", Uuid::new_v4()))
            .clone();
        if !secrets.contains_key(&portable_account) {
            let value = state
                .secret_store
                .get_json(&source_account)
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to read a required secret while creating backup",
                    )
                })?
                .ok_or_else(|| {
                    err(
                        McpErrorCode::ERR_SECRET_NOT_FOUND,
                        "a required stored credential is missing; backup was not created",
                    )
                })?;
            secrets.insert(
                portable_account.clone(),
                serde_json::to_string(&value).map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "failed to serialize a required secret",
                    )
                })?,
            );
        }
        storage.secret_ref = Some(portable_account);
    }

    if let Some(source_account) = portable_settings.auth_token_ref.clone() {
        let portable_account = source_to_portable
            .entry(source_account.clone())
            .or_insert_with(|| format!("backup/mcp-auth/{}", Uuid::new_v4()))
            .clone();
        if !secrets.contains_key(&portable_account) {
            let value = state
                .secret_store
                .get_json(&source_account)
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to read MCP authentication while creating backup",
                    )
                })?
                .ok_or_else(|| {
                    err(
                        McpErrorCode::ERR_SECRET_NOT_FOUND,
                        "configured MCP authentication is missing; backup was not created",
                    )
                })?;
            secrets.insert(
                portable_account.clone(),
                serde_json::to_string(&value).map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "failed to serialize MCP authentication",
                    )
                })?,
            );
        }
        portable_settings.auth_token_ref = Some(portable_account);
    }

    Ok(PortableBackupState {
        storages: portable_storages,
        mcp_settings: portable_settings,
        secrets,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewInput {
    pub passphrase: String,
    pub armored: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewOutput {
    pub preview_id: String,
    pub storage_count: usize,
    pub storage_additions: usize,
    pub storage_updates: usize,
    pub storage_removals: usize,
    pub has_mcp_settings: bool,
    pub has_app_settings: bool,
    pub has_workspaces: bool,
    pub has_secrets: bool,
    pub created_at: String,
    pub checksum_valid: bool,
    pub unsupported_version: bool,
    pub expires_in_seconds: u64,
}

#[derive(Clone)]
struct ValidatedRestore {
    storages: Vec<StorageRecord>,
    mcp_settings: Option<McpSettings>,
    app_settings: Option<AppSettings>,
    workspaces: Option<Vec<WorkspaceRecord>>,
    secrets: HashMap<String, Value>,
    created_at: String,
}

struct PendingRestore {
    validated: ValidatedRestore,
    base_digest: String,
    created_at: Instant,
    expires_at: Instant,
}

fn pending_restores() -> &'static Mutex<HashMap<String, PendingRestore>> {
    static PENDING: OnceLock<Mutex<HashMap<String, PendingRestore>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
pub async fn preview_recovery_restore(
    state: State<'_, AppState>,
    request: RestorePreviewInput,
) -> Result<RestorePreviewOutput, McpError> {
    let _transaction_guard = state.lifecycle_mutation.lock().await;
    let passphrase = SensitiveString::new(request.passphrase);
    let payload = backup::decrypt_backup(passphrase.as_str(), &request.armored)
        .map_err(map_backup_open_error)?;
    let validated = validate_payload(&payload)?;
    // A malformed current registry is a supported recovery trigger. It contributes
    // to the raw-state digest below, but cannot prevent previewing a valid backup.
    let existing = state.registry.load_all().unwrap_or_default();
    let base_digest = current_state_digest(&state, validated.secrets.keys())?;
    let existing_ids = existing
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let incoming_ids = validated
        .storages
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let additions = incoming_ids.difference(&existing_ids).count();
    let updates = incoming_ids.intersection(&existing_ids).count();
    let removals = existing_ids.difference(&incoming_ids).count();

    let preview_id = Uuid::new_v4().to_string();
    let mut pending = pending_restores().lock().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "restore preview state is unavailable",
        )
    })?;
    pending.retain(|_, item| item.expires_at > Instant::now());
    while pending.len() >= MAX_PENDING_RESTORE_PREVIEWS {
        let Some(oldest) = pending
            .iter()
            .min_by_key(|(_, item)| item.created_at)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        pending.remove(&oldest);
    }
    pending.insert(
        preview_id.clone(),
        PendingRestore {
            validated: validated.clone(),
            base_digest,
            created_at: Instant::now(),
            expires_at: Instant::now() + RESTORE_PREVIEW_TTL,
        },
    );

    Ok(RestorePreviewOutput {
        preview_id,
        storage_count: validated.storages.len(),
        storage_additions: additions,
        storage_updates: updates,
        storage_removals: removals,
        has_mcp_settings: validated.mcp_settings.is_some(),
        has_app_settings: validated.app_settings.is_some(),
        has_workspaces: validated.workspaces.is_some(),
        has_secrets: !validated.secrets.is_empty(),
        created_at: validated.created_at,
        checksum_valid: true,
        unsupported_version: false,
        expires_in_seconds: RESTORE_PREVIEW_TTL.as_secs(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestoreInput {
    pub preview_id: String,
    pub restore_mcp_settings: bool,
    pub restore_app_settings: bool,
    pub restore_workspaces: bool,
    pub restore_secrets: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestoreOutput {
    pub storages_restored: usize,
    pub mcp_settings_restored: bool,
    pub app_settings_restored: bool,
    pub workspaces_restored: bool,
    pub secrets_restored: usize,
}

struct RestoreSnapshot {
    storages: Vec<StorageRecord>,
    mcp_settings: McpSettings,
    app_settings: AppSettings,
    workspaces: Vec<WorkspaceRecord>,
    secrets: HashMap<String, Option<Value>>,
    /// Exact persisted bytes (or absence) used for rollback. Keeping these opaque
    /// allows recovery to replace malformed JSON without parsing it first.
    raw_files: BTreeMap<String, Option<Vec<u8>>>,
    runtime_was_running: bool,
}

#[tauri::command]
pub async fn apply_recovery_restore(
    state: State<'_, AppState>,
    request: ApplyRestoreInput,
) -> Result<ApplyRestoreOutput, McpError> {
    let _transaction_guard = state.lifecycle_mutation.lock().await;
    let pending = pending_restores()
        .lock()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "restore preview state is unavailable",
            )
        })?
        .remove(&request.preview_id)
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_IMPORT_PREVIEW_STALE,
                "restore preview is missing, expired, or already used",
            )
        })?;
    if pending.expires_at <= Instant::now() {
        return Err(err(
            McpErrorCode::ERR_IMPORT_PREVIEW_STALE,
            "restore preview has expired",
        ));
    }
    if current_state_digest(&state, pending.validated.secrets.keys())? != pending.base_digest {
        return Err(err(
            McpErrorCode::ERR_IMPORT_PREVIEW_STALE,
            "local configuration changed after restore preview",
        ));
    }

    let mut validated = pending.validated;
    if request.restore_mcp_settings && validated.mcp_settings.is_none() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "restore requested MCP settings that are not present in the backup",
        ));
    }
    if request.restore_app_settings && validated.app_settings.is_none() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "restore requested app settings that are not present in the backup",
        ));
    }
    if request.restore_workspaces && validated.workspaces.is_none() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "restore requested workspaces that are not present in the backup",
        ));
    }
    let local_workspaces = if request.restore_workspaces {
        Vec::new()
    } else {
        state.workspaces.load_all().map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to validate local workspace bindings before restore",
            )
        })?
    };
    validate_effective_restore_relationships(
        &validated.storages,
        validated.workspaces.as_deref(),
        &local_workspaces,
        request.restore_workspaces,
    )?;

    remap_restore_secret_accounts(&state, &mut validated, request.restore_mcp_settings)?;
    let selected_secrets = required_secret_accounts(
        &validated.storages,
        request
            .restore_mcp_settings
            .then_some(validated.mcp_settings.as_ref())
            .flatten(),
    );
    if !request.restore_secrets
        && selected_secrets
            .iter()
            .any(|name| validated.secrets.contains_key(name))
    {
        return Err(err(
            McpErrorCode::ERR_IMPORT_CONFIRMATION_REQUIRED,
            "restoring these storages/settings also requires restoring their referenced secrets",
        ));
    }

    let snapshot_secret_names = if request.restore_secrets {
        selected_secrets.iter().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let snapshot = snapshot_current_state(&state, snapshot_secret_names.into_iter()).await?;
    persist_restore_journal(&state, &snapshot)?;
    let result = apply_validated_restore(&state, &request, &validated, &selected_secrets).await;
    if let Err((stage, original)) = result {
        let rollback_errors = rollback_restore(&state, &snapshot).await;
        if rollback_errors.is_empty() {
            cleanup_restore_journal(&state);
        }
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "restore transaction failed",
            serde_json::json!({
                "stage": stage,
                "cause": original,
                "rollbackErrors": rollback_errors,
            }),
        ));
    }

    if finalize_restore_journal(&state).is_err() {
        let rollback_errors = rollback_restore(&state, &snapshot).await;
        if rollback_errors.is_empty() {
            cleanup_restore_journal(&state);
        }
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "restore transaction could not be committed",
            serde_json::json!({ "rollbackErrors": rollback_errors }),
        ));
    }

    Ok(ApplyRestoreOutput {
        storages_restored: validated.storages.len(),
        mcp_settings_restored: request.restore_mcp_settings && validated.mcp_settings.is_some(),
        app_settings_restored: request.restore_app_settings && validated.app_settings.is_some(),
        workspaces_restored: request.restore_workspaces && validated.workspaces.is_some(),
        secrets_restored: if request.restore_secrets {
            selected_secrets.len()
        } else {
            0
        },
    })
}

fn fresh_restore_secret_account(state: &AppState, category: &str) -> Result<String, McpError> {
    for _ in 0..8 {
        let candidate = format!("recovery/{category}/{}", Uuid::new_v4());
        let existing = state.secret_store.get_json(&candidate).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to reserve a destination credential account",
            )
        })?;
        if existing.is_none() {
            return Ok(candidate);
        }
    }
    Err(err(
        McpErrorCode::ERR_INTERNAL,
        "failed to allocate a collision-free destination credential account",
    ))
}

fn remap_restore_secret_accounts(
    state: &AppState,
    validated: &mut ValidatedRestore,
    include_mcp_auth: bool,
) -> Result<(), McpError> {
    let mut remapped = HashMap::<String, String>::new();
    let mut destination_secrets = HashMap::new();

    for storage in &mut validated.storages {
        let Some(portable_account) = storage.secret_ref.clone() else {
            continue;
        };
        let destination = match remapped.get(&portable_account) {
            Some(existing) => existing.clone(),
            None => {
                let destination = fresh_restore_secret_account(state, "storage")?;
                remapped.insert(portable_account.clone(), destination.clone());
                destination
            }
        };
        let value = validated
            .secrets
            .get(&portable_account)
            .cloned()
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_SECRET_NOT_FOUND,
                    "backup is missing a storage credential",
                )
            })?;
        destination_secrets.insert(destination.clone(), value);
        storage.secret_ref = Some(destination);
    }

    if include_mcp_auth {
        if let Some(settings) = &mut validated.mcp_settings {
            if let Some(portable_account) = settings.auth_token_ref.clone() {
                let destination = match remapped.get(&portable_account) {
                    Some(existing) => existing.clone(),
                    None => {
                        let destination = fresh_restore_secret_account(state, "mcp-auth")?;
                        remapped.insert(portable_account.clone(), destination.clone());
                        destination
                    }
                };
                let value = validated
                    .secrets
                    .get(&portable_account)
                    .cloned()
                    .ok_or_else(|| {
                        err(
                            McpErrorCode::ERR_SECRET_NOT_FOUND,
                            "backup is missing MCP authentication",
                        )
                    })?;
                destination_secrets.insert(destination.clone(), value);
                settings.auth_token_ref = Some(destination);
            }
        }
    }

    validated.secrets = destination_secrets;
    Ok(())
}

fn preflight_restored_operators(
    storages: &[StorageRecord],
    secrets: &HashMap<String, Value>,
) -> Result<(), &'static str> {
    for storage in storages {
        let mut resolved = storage.clone();
        if let Some(account) = &storage.secret_ref {
            let bundle = secrets
                .get(account)
                .ok_or("restored storage credential is unavailable during preflight")?;
            resolved.config = infimount_core::secrets::merge_secret_config(&storage.config, bundle);
            resolved.secret_ref = None;
        }
        infimount_mcp::opendal_adapter::build_operator_from_config(&resolved)
            .map_err(|_| "failed to construct a restored storage operator")?;
    }
    Ok(())
}

async fn apply_validated_restore(
    state: &AppState,
    request: &ApplyRestoreInput,
    validated: &ValidatedRestore,
    selected_secrets: &BTreeSet<String>,
) -> Result<(), (&'static str, &'static str)> {
    if request.restore_secrets {
        for (name, value) in &validated.secrets {
            if !selected_secrets.contains(name) {
                continue;
            }
            state
                .secret_store
                .put_json(name, value)
                .map_err(|_| ("secrets", "failed to persist a restored secret"))?;
        }
    }
    preflight_restored_operators(&validated.storages, &validated.secrets)
        .map_err(|message| ("operatorPreflight", message))?;
    state
        .registry
        .save_legacy_records_secure(validated.storages.clone())
        .map_err(|_| ("storages", "failed to persist restored storages"))?;
    if request.restore_mcp_settings {
        if let Some(settings) = &validated.mcp_settings {
            state
                .settings_store
                .save_atomic(settings)
                .map_err(|_| ("mcpSettings", "failed to persist restored MCP settings"))?;
        }
    }
    if request.restore_app_settings {
        if let Some(settings) = &validated.app_settings {
            state
                .app_settings_store
                .save_atomic(settings)
                .map_err(|_| ("appSettings", "failed to persist restored app settings"))?;
        }
    }
    if request.restore_workspaces {
        if let Some(workspaces) = &validated.workspaces {
            state
                .workspaces
                .replace_all(workspaces.clone())
                .map_err(|_| ("workspaces", "failed to persist restored workspaces"))?;
        }
    }
    state.operator_cache.clear();
    if state.startup_health().operational {
        state
            .ensure_runtime_from_settings_locked()
            .await
            .map_err(|_| ("runtime", "failed to reconcile MCP runtime"))?;
    } else {
        state
            .stop_http_server_locked()
            .await
            .map_err(|_| ("runtime", "failed to keep MCP runtime stopped"))?;
    }
    Ok(())
}

fn restore_raw_persisted_state(
    state: &AppState,
    raw_files: &BTreeMap<String, Option<Vec<u8>>>,
) -> Vec<&'static str> {
    let paths = persisted_state_paths(state);
    let mut errors = Vec::new();
    for (name, previous) in raw_files {
        let Some(path) = paths.get(name) else {
            if !errors.contains(&"persistedState") {
                errors.push("persistedState");
            }
            continue;
        };
        let failed = match previous {
            Some(bytes) => infimount_core::atomic_file::atomic_write_file(
                path,
                bytes,
                infimount_core::atomic_file::FILE_MODE,
            )
            .is_err(),
            None if path.exists() => std::fs::remove_file(path).is_err(),
            None => false,
        };
        if failed && !errors.contains(&"persistedState") {
            errors.push("persistedState");
        }
    }
    errors
}

async fn rollback_restore(state: &AppState, snapshot: &RestoreSnapshot) -> Vec<&'static str> {
    let mut errors = if snapshot.raw_files.is_empty() {
        // Compatibility for journals created by an earlier v0.8 development build.
        let mut errors = Vec::new();
        if state
            .workspaces
            .replace_all(snapshot.workspaces.clone())
            .is_err()
        {
            errors.push("workspaces");
        }
        if state
            .app_settings_store
            .save_atomic(&snapshot.app_settings)
            .is_err()
        {
            errors.push("appSettings");
        }
        if state
            .settings_store
            .save_atomic(&snapshot.mcp_settings)
            .is_err()
        {
            errors.push("mcpSettings");
        }
        if state
            .registry
            .save_legacy_records_secure(snapshot.storages.clone())
            .is_err()
        {
            errors.push("storages");
        }
        errors
    } else {
        restore_raw_persisted_state(state, &snapshot.raw_files)
    };
    for (name, previous) in &snapshot.secrets {
        let result = match previous {
            Some(value) => state.secret_store.put_json(name, value),
            None => state.secret_store.delete(name),
        };
        if result.is_err() && !errors.contains(&"secrets") {
            errors.push("secrets");
        }
    }
    state.operator_cache.clear();
    let runtime_result = if snapshot.runtime_was_running {
        state.ensure_runtime_from_settings_locked().await
    } else {
        state.stop_http_server_locked().await
    };
    if runtime_result.is_err() {
        errors.push("runtime");
    }
    errors
}

fn persisted_state_paths(state: &AppState) -> BTreeMap<String, PathBuf> {
    let config_dir = state
        .registry
        .path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    BTreeMap::from([
        ("storages".to_string(), state.registry.path().to_path_buf()),
        (
            "mcpSettings".to_string(),
            state.settings_store.path().to_path_buf(),
        ),
        (
            "appSettings".to_string(),
            config_dir.join("app_settings.json"),
        ),
        ("workspaces".to_string(), config_dir.join("workspaces.json")),
    ])
}

fn read_raw_persisted_state(
    state: &AppState,
) -> Result<BTreeMap<String, Option<Vec<u8>>>, McpError> {
    persisted_state_paths(state)
        .into_iter()
        .map(|(name, path)| {
            let bytes = if path.exists() {
                Some(std::fs::read(&path).map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "failed to snapshot persisted recovery state",
                    )
                })?)
            } else {
                None
            };
            Ok((name, bytes))
        })
        .collect()
}

async fn snapshot_current_state<'a>(
    state: &AppState,
    restored_secret_names: impl Iterator<Item = &'a String>,
) -> Result<RestoreSnapshot, McpError> {
    let raw_files = read_raw_persisted_state(state)?;
    // Typed values are useful for old journal compatibility, while raw_files is the
    // rollback authority and remains available even when current JSON is malformed.
    let storages = state.registry.load_all().unwrap_or_default();
    let mcp_settings = state.settings_store.load().unwrap_or_default();
    let app_settings = state.app_settings_store.load().unwrap_or_default();
    let workspaces = state.workspaces.load_all().unwrap_or_default();
    let mut secrets = HashMap::new();
    for name in restored_secret_names {
        let previous = state.secret_store.get_json(name).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to snapshot native secrets",
            )
        })?;
        secrets.insert(name.clone(), previous);
    }
    Ok(RestoreSnapshot {
        storages,
        mcp_settings,
        app_settings,
        workspaces,
        secrets,
        raw_files,
        runtime_was_running: state.is_http_running().await,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreJournal {
    version: u32,
    state: String,
    armored_snapshot: String,
}

fn restore_journal_path(state: &AppState) -> PathBuf {
    state
        .registry
        .path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("restore-transaction.json")
}

fn snapshot_payload(snapshot: &RestoreSnapshot) -> Result<BackupPayload, McpError> {
    let storages = snapshot
        .storages
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize restore snapshot",
            )
        })?;
    let secrets = snapshot
        .secrets
        .iter()
        .map(|(name, value)| {
            serde_json::to_string(value)
                .map(|serialized| (name.clone(), serialized))
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "failed to serialize restore secret snapshot",
                    )
                })
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    BackupPayload::new(
        storages,
        Some(serde_json::to_value(&snapshot.mcp_settings).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize restore snapshot",
            )
        })?),
        Some(serde_json::to_value(&snapshot.app_settings).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize restore snapshot",
            )
        })?),
        Some(serde_json::json!({
            "records": snapshot.workspaces,
            "rawFiles": snapshot.raw_files.iter().map(|(name, bytes)| {
                (name.clone(), bytes.as_ref().map(|value| BASE64.encode(value)))
            }).collect::<BTreeMap<_, _>>(),
            "runtimeWasRunning": snapshot.runtime_was_running,
        })),
        secrets,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to create restore snapshot",
        )
    })
}

fn snapshot_from_payload(payload: &BackupPayload) -> Result<RestoreSnapshot, McpError> {
    let storages = payload
        .storages
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))?;
    let mcp_settings = payload
        .mcp_settings
        .clone()
        .ok_or_else(|| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is incomplete"))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))
        })?;
    let app_settings = payload
        .app_settings
        .clone()
        .ok_or_else(|| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is incomplete"))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))
        })?;
    let workspace_snapshot = payload
        .workspaces
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is incomplete"))?;
    let workspaces = workspace_snapshot
        .get("records")
        .cloned()
        .ok_or_else(|| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is incomplete"))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))
        })?;
    let raw_files = workspace_snapshot
        .get("rawFiles")
        .cloned()
        .map(serde_json::from_value::<BTreeMap<String, Option<String>>>)
        .transpose()
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))?
        .unwrap_or_default()
        .into_iter()
        .map(|(name, encoded)| {
            let bytes = encoded
                .map(|value| BASE64.decode(value))
                .transpose()
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))?;
            Ok((name, bytes))
        })
        .collect::<Result<BTreeMap<_, _>, McpError>>()?;
    let runtime_was_running = workspace_snapshot
        .get("runtimeWasRunning")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let secrets = payload
        .secrets
        .iter()
        .map(|(name, value)| {
            serde_json::from_str::<Option<Value>>(value)
                .map(|value| (name.clone(), value))
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "restore snapshot is malformed"))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(RestoreSnapshot {
        storages,
        mcp_settings,
        app_settings,
        workspaces,
        secrets,
        raw_files,
        runtime_was_running,
    })
}

fn zeroize_json_value(value: &mut Value) {
    match value {
        Value::String(value) => backup::zeroize(value),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json_value),
        Value::Object(values) => values.values_mut().for_each(zeroize_json_value),
        _ => {}
    }
}

fn persist_restore_journal(state: &AppState, snapshot: &RestoreSnapshot) -> Result<(), McpError> {
    let path = restore_journal_path(state);
    if path.exists() {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "an interrupted restore must be recovered before another restore",
        ));
    }
    let mut passphrase = format!("{}{}", Uuid::new_v4(), Uuid::new_v4());
    let mut key_bundle = serde_json::json!({ "passphrase": passphrase });
    let key_result = state
        .secret_store
        .put_json(RESTORE_JOURNAL_KEY_ACCOUNT, &key_bundle)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to protect restore recovery state",
            )
        });
    zeroize_json_value(&mut key_bundle);
    key_result?;
    let result = (|| {
        let payload = snapshot_payload(snapshot)?;
        let armored = backup::encrypt_backup(&passphrase, &payload).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to encrypt restore recovery state",
            )
        })?;
        let journal = RestoreJournal {
            version: RESTORE_JOURNAL_VERSION,
            state: "pending".to_string(),
            armored_snapshot: armored,
        };
        let bytes = serde_json::to_vec(&journal).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize restore recovery state",
            )
        })?;
        infimount_core::atomic_file::atomic_write_file(
            &path,
            &bytes,
            infimount_core::atomic_file::FILE_MODE,
        )
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to persist restore recovery state",
            )
        })
    })();
    backup::zeroize(&mut passphrase);
    if result.is_err() {
        let _ = state.secret_store.delete(RESTORE_JOURNAL_KEY_ACCOUNT);
    }
    result
}

fn finalize_restore_journal(state: &AppState) -> Result<(), McpError> {
    let journal = RestoreJournal {
        version: RESTORE_JOURNAL_VERSION,
        state: "committed".to_string(),
        armored_snapshot: String::new(),
    };
    let bytes = serde_json::to_vec(&journal).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to commit restore transaction",
        )
    })?;
    infimount_core::atomic_file::atomic_write_file(
        &restore_journal_path(state),
        &bytes,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to commit restore transaction",
        )
    })?;
    cleanup_restore_journal(state);
    Ok(())
}

fn cleanup_restore_journal(state: &AppState) {
    let _ = std::fs::remove_file(restore_journal_path(state));
    let _ = state.secret_store.delete(RESTORE_JOURNAL_KEY_ACCOUNT);
}

pub(crate) fn recover_interrupted_restore(state: &AppState) -> Result<(), McpError> {
    let path = restore_journal_path(state);
    if !path.exists() {
        let _ = state.secret_store.delete(RESTORE_JOURNAL_KEY_ACCOUNT);
        return Ok(());
    }
    let bytes = std::fs::read(&path).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to read restore recovery state",
        )
    })?;
    let journal: RestoreJournal = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "restore recovery state is malformed",
        )
    })?;
    if journal.version != RESTORE_JOURNAL_VERSION {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "restore recovery state has an unsupported version",
        ));
    }
    if journal.state == "committed" {
        cleanup_restore_journal(state);
        return Ok(());
    }
    let mut key_bundle = state
        .secret_store
        .get_json(RESTORE_JOURNAL_KEY_ACCOUNT)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to open restore recovery state",
            )
        })?
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "restore recovery key is missing",
            )
        })?;
    let passphrase_value = key_bundle
        .get("passphrase")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "restore recovery key is missing",
            )
        });
    zeroize_json_value(&mut key_bundle);
    let mut passphrase = passphrase_value?;
    let payload = backup::decrypt_backup(&passphrase, &journal.armored_snapshot).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to open restore recovery state",
        )
    });
    backup::zeroize(&mut passphrase);
    let snapshot = snapshot_from_payload(&payload?)?;
    restore_snapshot_sections(state, &snapshot)?;
    cleanup_restore_journal(state);
    Ok(())
}

fn restore_snapshot_sections(state: &AppState, snapshot: &RestoreSnapshot) -> Result<(), McpError> {
    if snapshot.raw_files.is_empty() {
        state
            .registry
            .save_legacy_records_secure(snapshot.storages.clone())
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    "failed to recover storage registry",
                )
            })?;
        state
            .settings_store
            .save_atomic(&snapshot.mcp_settings)
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to recover MCP settings"))?;
        state
            .app_settings_store
            .save_atomic(&snapshot.app_settings)
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to recover app settings"))?;
        state
            .workspaces
            .replace_all(snapshot.workspaces.clone())
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to recover workspaces"))?;
    } else {
        let errors = restore_raw_persisted_state(state, &snapshot.raw_files);
        if !errors.is_empty() {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "failed to recover persisted configuration",
            ));
        }
    }
    for (name, value) in &snapshot.secrets {
        let result = match value {
            Some(value) => state.secret_store.put_json(name, value),
            None => state.secret_store.delete(name),
        };
        result.map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to recover native secrets",
            )
        })?;
    }
    state.operator_cache.clear();
    Ok(())
}

fn validate_payload(payload: &BackupPayload) -> Result<ValidatedRestore, McpError> {
    let storages = payload
        .storages
        .iter()
        .map(|value| serde_json::from_value::<StorageRecord>(value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "backup contains malformed storage data",
            )
        })?;
    let mcp_settings = payload
        .mcp_settings
        .as_ref()
        .map(|value| serde_json::from_value::<McpSettings>(value.clone()))
        .transpose()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "backup contains malformed MCP settings",
            )
        })?;
    let app_settings = payload
        .app_settings
        .as_ref()
        .map(|value| serde_json::from_value::<AppSettings>(value.clone()))
        .transpose()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "backup contains malformed app settings",
            )
        })?;
    let workspaces = payload
        .workspaces
        .as_ref()
        .map(|value| serde_json::from_value::<Vec<WorkspaceRecord>>(value.clone()))
        .transpose()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "backup contains malformed workspace data",
            )
        })?;
    validate_restore_relationships(&storages, workspaces.as_deref())?;

    let secrets = payload
        .secrets
        .iter()
        .map(|(name, serialized)| {
            if name.trim().is_empty() {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "backup contains an invalid secret reference",
                ));
            }
            serde_json::from_str::<Value>(serialized)
                .map(|value| (name.clone(), value))
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "backup contains malformed secret data",
                    )
                })
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let required_accounts = required_secret_accounts(&storages, mcp_settings.as_ref());
    for required in &required_accounts {
        if !secrets.contains_key(required) {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "backup is missing a credential referenced by its configuration",
            ));
        }
    }
    if secrets.keys().any(|name| !required_accounts.contains(name)) {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "backup contains an unreferenced secret account",
        ));
    }

    Ok(ValidatedRestore {
        storages,
        mcp_settings,
        app_settings,
        workspaces,
        secrets,
        created_at: payload.created_at.clone(),
    })
}

fn workspace_access_mode(profile: &str) -> Option<McpAccessMode> {
    match profile {
        "none" => Some(McpAccessMode::None),
        "read_only" => Some(McpAccessMode::ReadOnly),
        "read_write" => Some(McpAccessMode::ReadWrite),
        _ => None,
    }
}

fn validate_restore_relationships(
    storages: &[StorageRecord],
    workspaces: Option<&[WorkspaceRecord]>,
) -> Result<(), McpError> {
    let malformed = |message: &'static str| err(McpErrorCode::ERR_INTERNAL, message);
    let mut storage_ids = BTreeSet::new();
    let mut storage_names = BTreeSet::new();
    let mut policy_rule_ids = BTreeSet::new();

    for storage in storages {
        if storage.schema_version != STORAGE_RECORD_SCHEMA_VERSION
            || storage.id.trim().is_empty()
            || storage.name.trim().is_empty()
            || !storage_ids.insert(storage.id.clone())
            || !storage_names.insert(storage.name.trim().to_lowercase())
        {
            return Err(malformed(
                "backup contains unsupported, empty, or duplicate storage identity data",
            ));
        }
        if storage.mcp_policy.version != MCP_POLICY_VERSION {
            return Err(malformed(
                "backup contains an unsupported MCP policy version",
            ));
        }
        let mut normalized_policy = storage.mcp_policy.clone();
        normalize_storage_policy(&mut normalized_policy).map_err(|_| {
            malformed("backup contains a malformed or ambiguous MCP storage policy")
        })?;
        if normalized_policy != storage.mcp_policy {
            return Err(malformed(
                "backup contains a non-canonical MCP storage policy",
            ));
        }
        for rule in &storage.mcp_policy.rules {
            if rule.id.trim().is_empty() || !policy_rule_ids.insert(rule.id.clone()) {
                return Err(malformed("backup contains duplicate MCP policy rule IDs"));
            }
        }
    }

    let workspaces = workspaces.unwrap_or(&[]);
    let mut workspace_ids = BTreeSet::new();
    let mut workspace_names = BTreeSet::new();
    for workspace in workspaces {
        if workspace.schema_version != WORKSPACE_RECORD_SCHEMA_VERSION
            || workspace.id.trim().is_empty()
            || workspace.name.trim().is_empty()
            || !workspace_ids.insert(workspace.id.clone())
            || !workspace_names.insert(workspace.name.trim().to_lowercase())
        {
            return Err(malformed(
                "backup contains unsupported, empty, or duplicate workspace identity data",
            ));
        }
        validate_workspace_metadata(workspace)
            .map_err(|_| malformed("backup contains malformed workspace metadata"))?;
        let storage = storages
            .iter()
            .find(|storage| storage.id == workspace.storage_id)
            .ok_or_else(|| malformed("backup workspace references a missing storage"))?;
        let backup_fingerprint =
            infimount_mcp::storage_namespace::storage_namespace_fingerprint(storage)
                .map_err(|_| malformed("backup storage namespace could not be fingerprinted"))?;
        if workspace.storage_namespace_fingerprint != backup_fingerprint {
            return Err(malformed(
                "backup workspace namespace does not match the referenced storage",
            ));
        }
        let expected_access = workspace_access_mode(&workspace.access_profile)
            .ok_or_else(|| malformed("backup workspace has an invalid access profile"))?;
        let expected_prefix = normalize_policy_path(&workspace.root_path)
            .map_err(|_| malformed("backup workspace has an invalid root path"))?;
        for other in workspaces
            .iter()
            .filter(|other| other.id != workspace.id && other.storage_id == workspace.storage_id)
        {
            let other_prefix = normalize_policy_path(&other.root_path)
                .map_err(|_| malformed("backup workspace has an invalid root path"))?;
            if expected_prefix == other_prefix
                || expected_prefix.starts_with(&format!("{other_prefix}/"))
                || other_prefix.starts_with(&format!("{expected_prefix}/"))
            {
                return Err(malformed("backup contains overlapping workspace roots"));
            }
        }
        match workspace.policy_rule_id.as_deref() {
            Some(rule_id) => {
                let rule = storage
                    .mcp_policy
                    .rules
                    .iter()
                    .find(|rule| rule.id == rule_id)
                    .ok_or_else(|| malformed("backup workspace policy linkage is missing"))?;
                if rule.prefix != expected_prefix
                    || rule.access != expected_access
                    || rule.confirmation_rules.is_some()
                    || !matches!(
                        &rule.source,
                        McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
                    )
                {
                    return Err(malformed("backup workspace policy linkage is inconsistent"));
                }
            }
            None => {
                if expected_access != McpAccessMode::None
                    || storage.mcp_policy.rules.iter().any(|rule| {
                        matches!(
                            &rule.source,
                            McpRuleSource::Workspace { workspace_id }
                                if workspace_id == &workspace.id
                        )
                    })
                {
                    return Err(malformed("backup workspace policy linkage is inconsistent"));
                }
            }
        }
    }

    for storage in storages {
        for rule in &storage.mcp_policy.rules {
            if let McpRuleSource::Workspace { workspace_id } = &rule.source {
                let workspace = workspaces
                    .iter()
                    .find(|workspace| workspace.id == *workspace_id)
                    .ok_or_else(|| malformed("backup policy references a missing workspace"))?;
                if workspace.storage_id != storage.id
                    || workspace.policy_rule_id.as_deref() != Some(rule.id.as_str())
                    || rule.confirmation_rules.is_some()
                {
                    return Err(malformed("backup policy workspace linkage is inconsistent"));
                }
            }
        }
    }
    Ok(())
}

fn validate_effective_restore_relationships(
    incoming_storages: &[StorageRecord],
    incoming_workspaces: Option<&[WorkspaceRecord]>,
    local_workspaces: &[WorkspaceRecord],
    restore_workspaces: bool,
) -> Result<(), McpError> {
    let effective_workspaces = if restore_workspaces {
        incoming_workspaces.ok_or_else(|| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "restore requested workspaces that are not present in the backup",
            )
        })?
    } else {
        local_workspaces
    };
    validate_restore_relationships(incoming_storages, Some(effective_workspaces)).map_err(|_| {
        err(
            McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
            "restore would break an effective workspace storage or policy binding",
        )
    })
}

fn required_secret_accounts(
    storages: &[StorageRecord],
    mcp_settings: Option<&McpSettings>,
) -> BTreeSet<String> {
    let mut names = storages
        .iter()
        .filter_map(|storage| storage.secret_ref.clone())
        .collect::<BTreeSet<_>>();
    if let Some(reference) = mcp_settings.and_then(|settings| settings.auth_token_ref.clone()) {
        names.insert(reference);
    }
    names
}

fn current_state_digest<'a>(
    state: &AppState,
    additional_secret_names: impl Iterator<Item = &'a String>,
) -> Result<String, McpError> {
    let raw_files = read_raw_persisted_state(state)?;
    let storages = state.registry.load_all().unwrap_or_default();
    let mcp_settings = state.settings_store.load().unwrap_or_default();
    let mut secret_names = required_secret_accounts(&storages, Some(&mcp_settings));
    secret_names.extend(additional_secret_names.cloned());
    let mut secret_digests = BTreeMap::new();
    for name in secret_names {
        let value = state.secret_store.get_json(&name).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to read local secret revision",
            )
        })?;
        let digest = value
            .map(|value| {
                serde_json::to_vec(&value)
                    .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                    .map_err(|_| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            "failed to compute local secret revision",
                        )
                    })
            })
            .transpose()?;
        secret_digests.insert(name, digest);
    }
    let file_digests = raw_files
        .into_iter()
        .map(|(name, bytes)| {
            (
                name,
                bytes.map(|value| format!("{:x}", Sha256::digest(value))),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "persistedFiles": file_digests,
        "secretDigests": secret_digests,
    }))
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to compute restore revision",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn map_backup_open_error(error: backup::BackupError) -> McpError {
    let reason = match error {
        backup::BackupError::Serialization(message) if message.contains("unsupported backup") => {
            "unsupported_version"
        }
        backup::BackupError::ChecksumMismatch(_) => "checksum_mismatch",
        _ => "decryption_failed",
    };
    err_with_details(
        McpErrorCode::ERR_BACKUP_DECRYPTION_FAILED,
        "failed to open recovery backup",
        serde_json::json!({ "reason": reason }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::SecretStore;

    #[test]
    fn required_accounts_use_persisted_references() {
        let mut storage = StorageRecord::new("test".into(), "local".into(), serde_json::json!({}));
        storage.secret_ref = Some("storage/example".into());
        let settings = McpSettings {
            auth_token_ref: Some("mcp/example".into()),
            ..McpSettings::default()
        };
        let accounts = required_secret_accounts(&[storage], Some(&settings));
        assert_eq!(
            accounts.into_iter().collect::<Vec<_>>(),
            vec!["mcp/example", "storage/example"]
        );
    }

    #[test]
    fn strict_validation_rejects_missing_referenced_secret() {
        let mut storage = StorageRecord::new("test".into(), "local".into(), serde_json::json!({}));
        storage.secret_ref = Some("storage/missing".into());
        let payload = BackupPayload::new(
            vec![serde_json::to_value(storage).unwrap()],
            None,
            None,
            None,
            HashMap::new(),
        )
        .unwrap();
        assert!(validate_payload(&payload).is_err());
    }

    #[test]
    fn strict_validation_rejects_unreferenced_secret_account() {
        let mut secrets = HashMap::new();
        secrets.insert("unreferenced/account".into(), "{}".into());
        let payload = BackupPayload::new(Vec::new(), None, None, None, secrets).unwrap();
        assert!(validate_payload(&payload).is_err());
    }

    fn test_state(
        dir: &tempfile::TempDir,
    ) -> (
        AppState,
        std::sync::Arc<infimount_core::secrets::MemorySecretStore>,
    ) {
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let state = AppState::new_for_test(dir.path(), secrets.clone());
        state
            .registry
            .save_legacy_records_secure(Vec::new())
            .unwrap();
        state
            .settings_store
            .save_atomic(&McpSettings::default())
            .unwrap();
        state
            .app_settings_store
            .save_atomic(&AppSettings::default())
            .unwrap();
        state.workspaces.replace_all(Vec::new()).unwrap();
        (state, secrets)
    }

    #[test]
    fn stale_digest_changes_when_a_referenced_secret_value_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (state, secrets) = test_state(&dir);
        let mut storage = StorageRecord::new("Local".into(), "local".into(), serde_json::json!({}));
        storage.secret_ref = Some("storage/local".into());
        state
            .registry
            .save_legacy_records_secure(vec![storage])
            .unwrap();
        secrets
            .put_json("storage/local", &serde_json::json!({"token": "first"}))
            .unwrap();
        let first = current_state_digest(&state, std::iter::empty()).unwrap();
        secrets
            .put_json("storage/local", &serde_json::json!({"token": "second"}))
            .unwrap();
        assert_ne!(
            first,
            current_state_digest(&state, std::iter::empty()).unwrap()
        );
    }

    #[test]
    fn stale_digest_tracks_incoming_secret_accounts_not_yet_referenced_locally() {
        let dir = tempfile::tempdir().unwrap();
        let (state, secrets) = test_state(&dir);
        let incoming = ["storage/incoming".to_string()];
        secrets
            .put_json("storage/incoming", &serde_json::json!({"token": "first"}))
            .unwrap();
        let first = current_state_digest(&state, incoming.iter()).unwrap();
        secrets
            .put_json("storage/incoming", &serde_json::json!({"token": "second"}))
            .unwrap();
        assert_ne!(
            first,
            current_state_digest(&state, incoming.iter()).unwrap()
        );
    }

    #[tokio::test]
    async fn portable_backup_and_cross_machine_restore_remap_secret_accounts() {
        let source_dir = tempfile::tempdir().unwrap();
        let (source_state, source_secrets) = test_state(&source_dir);
        source_secrets
            .put_json(
                "machine-a/storage/account",
                &serde_json::json!({"accessKeyId": "fixture-access"}),
            )
            .unwrap();
        source_secrets
            .put_json(
                "machine-a/mcp/account",
                &serde_json::json!({"token": "fixture-token"}),
            )
            .unwrap();
        let mut storage = StorageRecord::new(
            "Portable".into(),
            "local".into(),
            serde_json::json!({"root": "/tmp"}),
        );
        storage.secret_ref = Some("machine-a/storage/account".into());
        let settings = McpSettings {
            auth_token_ref: Some("machine-a/mcp/account".into()),
            ..McpSettings::default()
        };
        let portable = make_portable_backup_state(&source_state, &[storage], &settings).unwrap();
        assert!(portable.storages[0]
            .secret_ref
            .as_deref()
            .unwrap()
            .starts_with("backup/storage/"));
        assert!(portable
            .mcp_settings
            .auth_token_ref
            .as_deref()
            .unwrap()
            .starts_with("backup/mcp-auth/"));
        assert!(!portable.secrets.contains_key("machine-a/storage/account"));

        let destination_dir = tempfile::tempdir().unwrap();
        let (destination_state, destination_secrets) = test_state(&destination_dir);
        destination_secrets
            .put_json(
                "machine-a/storage/account",
                &serde_json::json!({"accessKeyId": "destination-existing"}),
            )
            .unwrap();
        let mut validated = ValidatedRestore {
            storages: portable.storages,
            mcp_settings: Some(portable.mcp_settings),
            app_settings: None,
            workspaces: None,
            secrets: portable
                .secrets
                .into_iter()
                .map(|(key, value)| (key, serde_json::from_str(&value).unwrap()))
                .collect(),
            created_at: String::new(),
        };
        remap_restore_secret_accounts(&destination_state, &mut validated, true).unwrap();
        let storage_account = validated.storages[0].secret_ref.as_deref().unwrap();
        let auth_account = validated
            .mcp_settings
            .as_ref()
            .unwrap()
            .auth_token_ref
            .as_deref()
            .unwrap();
        assert!(storage_account.starts_with("recovery/storage/"));
        assert!(auth_account.starts_with("recovery/mcp-auth/"));
        assert_ne!(storage_account, "machine-a/storage/account");
        assert_ne!(storage_account, auth_account);
        assert_eq!(validated.secrets.len(), 2);
        let request = ApplyRestoreInput {
            preview_id: String::new(),
            restore_mcp_settings: true,
            restore_app_settings: false,
            restore_workspaces: false,
            restore_secrets: true,
        };
        let selected =
            required_secret_accounts(&validated.storages, validated.mcp_settings.as_ref());
        apply_validated_restore(&destination_state, &request, &validated, &selected)
            .await
            .unwrap();
        let persisted = destination_state.registry.load_all().unwrap();
        assert_eq!(persisted[0].secret_ref.as_deref(), Some(storage_account));
        assert!(destination_secrets
            .get_json(storage_account)
            .unwrap()
            .is_some());
        assert!(destination_secrets
            .get_json(auth_account)
            .unwrap()
            .is_some());
        assert_eq!(
            destination_secrets
                .get_json("machine-a/storage/account")
                .unwrap()
                .unwrap(),
            serde_json::json!({"accessKeyId": "destination-existing"})
        );
    }

    #[test]
    fn strict_validation_rejects_duplicate_storage_names() {
        let first = StorageRecord::new("Duplicate".into(), "local".into(), serde_json::json!({}));
        let second =
            StorageRecord::new(" duplicate ".into(), "local".into(), serde_json::json!({}));
        let payload = BackupPayload::new(
            vec![
                serde_json::to_value(first).unwrap(),
                serde_json::to_value(second).unwrap(),
            ],
            None,
            None,
            None,
            HashMap::new(),
        )
        .unwrap();
        assert!(validate_payload(&payload).is_err());
    }

    #[test]
    fn strict_validation_rejects_workspace_with_missing_storage() {
        let workspace = WorkspaceRecord {
            id: "workspace-1".into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: "missing-storage".into(),
            name: "Workspace".into(),
            root_path: "/workspace".into(),
            template_id: "custom".into(),
            access_profile: "read_only".into(),
            policy_rule_id: Some("workspace:workspace-1".into()),
            storage_namespace_fingerprint: "fixture".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            memory_files: Vec::new(),
            checkpoint_ids: Vec::new(),
        };
        let payload = BackupPayload::new(
            Vec::new(),
            None,
            None,
            Some(serde_json::to_value(vec![workspace]).unwrap()),
            HashMap::new(),
        )
        .unwrap();
        assert!(validate_payload(&payload).is_err());
    }

    fn workspace_bound_restore_fixture() -> (StorageRecord, WorkspaceRecord) {
        let mut storage = StorageRecord::new(
            "Workspace storage".into(),
            "local".into(),
            serde_json::json!({ "root": "/tmp" }),
        );
        let workspace = WorkspaceRecord {
            id: "workspace-1".into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Workspace".into(),
            root_path: "workspace".into(),
            template_id: "custom".into(),
            access_profile: "read_only".into(),
            policy_rule_id: Some("workspace:workspace-1".into()),
            storage_namespace_fingerprint:
                infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
                    .expect("storage namespace fingerprint"),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            memory_files: Vec::new(),
            checkpoint_ids: Vec::new(),
        };
        storage
            .mcp_policy
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: workspace.policy_rule_id.clone().unwrap(),
                prefix: workspace.root_path.clone(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: workspace.id.clone(),
                },
                confirmation_rules: None,
            });
        (storage, workspace)
    }

    #[test]
    fn strict_validation_rejects_overlapping_workspace_roots() {
        let (storage, workspace) = workspace_bound_restore_fixture();
        let nested = WorkspaceRecord {
            id: "workspace-2".into(),
            root_path: "workspace/nested".into(),
            policy_rule_id: Some("workspace:workspace-2".into()),
            ..workspace.clone()
        };
        assert!(validate_effective_restore_relationships(
            std::slice::from_ref(&storage),
            Some(&[workspace, nested]),
            &[],
            true,
        )
        .is_err());
    }

    #[test]
    fn selective_restore_preserves_local_workspace_policy_binding() {
        let (storage, workspace) = workspace_bound_restore_fixture();
        validate_effective_restore_relationships(
            std::slice::from_ref(&storage),
            None,
            std::slice::from_ref(&workspace),
            false,
        )
        .unwrap();

        let removed = validate_effective_restore_relationships(
            &[],
            None,
            std::slice::from_ref(&workspace),
            false,
        )
        .expect_err("selective restore must preserve referenced storage");
        assert_eq!(removed.code, McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH);

        let mut missing_rule = storage.clone();
        missing_rule.mcp_policy.rules.clear();
        assert!(validate_effective_restore_relationships(
            &[missing_rule],
            None,
            std::slice::from_ref(&workspace),
            false,
        )
        .is_err());

        let mut changed_prefix = storage.clone();
        changed_prefix.mcp_policy.rules[0].prefix = "other".into();
        assert!(validate_effective_restore_relationships(
            &[changed_prefix],
            None,
            std::slice::from_ref(&workspace),
            false,
        )
        .is_err());

        let mut changed_access = storage.clone();
        changed_access.mcp_policy.rules[0].access = McpAccessMode::ReadWrite;
        assert!(validate_effective_restore_relationships(
            &[changed_access],
            None,
            std::slice::from_ref(&workspace),
            false,
        )
        .is_err());

        let mut disabled_confirmation = storage;
        disabled_confirmation.mcp_policy.rules[0].confirmation_rules =
            Some(infimount_mcp::policy::McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                require_for_delete: false,
                require_for_version_delete: false,
                require_for_presign: false,
                require_for_cross_storage_copy: false,
            });
        assert!(validate_effective_restore_relationships(
            &[disabled_confirmation],
            None,
            &[workspace],
            false,
        )
        .is_err());
    }

    #[test]
    fn full_restore_validates_incoming_workspaces_instead_of_local_records() {
        let (storage, workspace) = workspace_bound_restore_fixture();
        let unrelated_local = WorkspaceRecord {
            id: "local-only".into(),
            storage_id: "removed-local-storage".into(),
            policy_rule_id: Some("workspace:local-only".into()),
            ..workspace.clone()
        };
        validate_effective_restore_relationships(
            std::slice::from_ref(&storage),
            Some(std::slice::from_ref(&workspace)),
            std::slice::from_ref(&unrelated_local),
            true,
        )
        .unwrap();

        let mut disabled_confirmation = storage;
        disabled_confirmation.mcp_policy.rules[0].confirmation_rules =
            Some(infimount_mcp::policy::McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                require_for_delete: false,
                require_for_version_delete: false,
                require_for_presign: false,
                require_for_cross_storage_copy: false,
            });
        assert!(validate_effective_restore_relationships(
            &[disabled_confirmation],
            Some(&[workspace]),
            &[unrelated_local],
            true,
        )
        .is_err());
    }

    #[test]
    fn operator_preflight_rejects_unconstructable_backend() {
        let storage = StorageRecord::new(
            "Unsupported".into(),
            "not-a-backend".into(),
            serde_json::json!({}),
        );
        assert!(preflight_restored_operators(&[storage], &HashMap::new()).is_err());
    }

    #[tokio::test]
    async fn selective_restore_does_not_replace_mcp_auth_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (state, secrets) = test_state(&dir);
        secrets
            .put_json(
                "storage/local",
                &serde_json::json!({"token": "old-storage"}),
            )
            .unwrap();
        secrets
            .put_json("mcp/auth-token", &serde_json::json!({"token": "old-auth"}))
            .unwrap();
        let mut storage = StorageRecord::new(
            "Local".into(),
            "local".into(),
            serde_json::json!({"root": dir.path().to_string_lossy()}),
        );
        storage.secret_ref = Some("storage/local".into());
        let validated = ValidatedRestore {
            storages: vec![storage],
            mcp_settings: Some(McpSettings {
                auth_token_ref: Some("mcp/auth-token".into()),
                ..McpSettings::default()
            }),
            app_settings: None,
            workspaces: None,
            secrets: HashMap::from([
                (
                    "storage/local".into(),
                    serde_json::json!({"token": "new-storage"}),
                ),
                (
                    "mcp/auth-token".into(),
                    serde_json::json!({"token": "new-auth"}),
                ),
            ]),
            created_at: String::new(),
        };
        let request = ApplyRestoreInput {
            preview_id: String::new(),
            restore_mcp_settings: false,
            restore_app_settings: false,
            restore_workspaces: false,
            restore_secrets: true,
        };
        let selected = required_secret_accounts(&validated.storages, None);
        apply_validated_restore(&state, &request, &validated, &selected)
            .await
            .unwrap();
        assert_eq!(
            secrets.get_json("storage/local").unwrap().unwrap(),
            serde_json::json!({"token": "new-storage"})
        );
        assert_eq!(
            secrets.get_json("mcp/auth-token").unwrap().unwrap(),
            serde_json::json!({"token": "old-auth"})
        );
    }

    #[tokio::test]
    async fn durable_snapshot_recovers_all_persisted_sections() {
        let dir = tempfile::tempdir().unwrap();
        let (state, secrets) = test_state(&dir);
        secrets
            .put_json("storage/restore", &serde_json::json!({"token": "before"}))
            .unwrap();
        let secret_names = ["storage/restore".to_string()];
        let snapshot = snapshot_current_state(&state, secret_names.iter())
            .await
            .unwrap();
        persist_restore_journal(&state, &snapshot).unwrap();

        state
            .settings_store
            .save_atomic(&McpSettings {
                enabled: true,
                ..McpSettings::default()
            })
            .unwrap();
        secrets
            .put_json("storage/restore", &serde_json::json!({"token": "after"}))
            .unwrap();
        recover_interrupted_restore(&state).unwrap();

        assert!(!state.settings_store.load().unwrap().enabled);
        assert_eq!(
            secrets.get_json("storage/restore").unwrap().unwrap(),
            serde_json::json!({"token": "before"})
        );
        assert!(!restore_journal_path(&state).exists());
    }

    #[tokio::test]
    async fn malformed_registry_can_be_replaced_and_rolled_back_as_opaque_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let storage_root = dir.path().join("restored-root");
        std::fs::create_dir_all(&storage_root).unwrap();
        let (state, _) = test_state(&dir);
        let corrupt = b"{ definitely-not-json".to_vec();
        std::fs::write(state.registry.path(), &corrupt).unwrap();
        assert!(state.registry.load_all().is_err());

        let digest = current_state_digest(&state, std::iter::empty()).unwrap();
        assert!(!digest.is_empty());
        let snapshot = snapshot_current_state(&state, std::iter::empty())
            .await
            .unwrap();
        assert_eq!(
            snapshot.raw_files.get("storages"),
            Some(&Some(corrupt.clone()))
        );

        let restored = StorageRecord::new(
            "Recovered".into(),
            "local".into(),
            serde_json::json!({"root": storage_root}),
        );
        let validated = ValidatedRestore {
            storages: vec![restored],
            mcp_settings: None,
            app_settings: None,
            workspaces: None,
            secrets: HashMap::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let request = ApplyRestoreInput {
            preview_id: String::new(),
            restore_mcp_settings: false,
            restore_app_settings: false,
            restore_workspaces: false,
            restore_secrets: false,
        };
        apply_validated_restore(&state, &request, &validated, &BTreeSet::new())
            .await
            .unwrap();
        assert_eq!(state.registry.load_all().unwrap().len(), 1);

        assert!(rollback_restore(&state, &snapshot).await.is_empty());
        assert_eq!(std::fs::read(state.registry.path()).unwrap(), corrupt);
        assert!(state.registry.load_all().is_err());
    }

    #[tokio::test]
    async fn apply_and_rollback_reconcile_http_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _) = test_state(&dir);
        let snapshot = snapshot_current_state(&state, std::iter::empty())
            .await
            .unwrap();
        let validated = ValidatedRestore {
            storages: Vec::new(),
            mcp_settings: Some(McpSettings {
                enabled: true,
                transport: infimount_mcp::settings::McpTransport::Http,
                bind_address: "127.0.0.1".into(),
                port: 0,
                ..McpSettings::default()
            }),
            app_settings: None,
            workspaces: None,
            secrets: HashMap::new(),
            created_at: String::new(),
        };
        let request = ApplyRestoreInput {
            preview_id: String::new(),
            restore_mcp_settings: true,
            restore_app_settings: false,
            restore_workspaces: false,
            restore_secrets: false,
        };
        apply_validated_restore(&state, &request, &validated, &BTreeSet::new())
            .await
            .unwrap();
        assert!(state.is_http_running().await);
        assert!(rollback_restore(&state, &snapshot).await.is_empty());
        assert!(!state.is_http_running().await);
        assert!(!state.settings_store.load().unwrap().enabled);
    }
}
