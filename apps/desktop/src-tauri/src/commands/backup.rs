use infimount_core::backup::{self, BackupPayload};
use infimount_mcp::errors::{err, McpErrorCode, McpError};
use infimount_mcp::settings::McpSettings;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_settings::AppSettings;
use crate::state::AppState;

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
}

#[tauri::command]
pub fn create_recovery_backup(
    state: State<'_, AppState>,
    request: CreateBackupInput,
) -> Result<CreateBackupOutput, McpError> {
    let storages = state.registry.load_all().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to load storage registry: {e}"),
        )
    })?;
    let storage_values: Vec<serde_json::Value> = storages
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();

    let mcp_settings = state.settings_store.load().ok().map(|s: McpSettings| {
        serde_json::to_value(&s).unwrap_or_default()
    });
    let app_settings = state
        .app_settings_store
        .load()
        .ok()
        .map(|s| serde_json::to_value(s).unwrap_or_default());

    let payload = BackupPayload::new(storage_values, mcp_settings, app_settings).map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to create backup payload: {e}"),
        )
    })?;

    let armored = backup::encrypt_backup(&request.passphrase, &payload).map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to encrypt backup: {e}"),
        )
    })?;

    Ok(CreateBackupOutput {
        armored,
        storage_count: storages.len(),
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
    pub storage_count: usize,
    pub has_mcp_settings: bool,
    pub has_app_settings: bool,
    pub created_at: String,
    pub checksum_valid: bool,
}

#[tauri::command]
pub fn preview_recovery_restore(
    request: RestorePreviewInput,
) -> Result<RestorePreviewOutput, McpError> {
    let payload = backup::decrypt_backup(&request.passphrase, &request.armored).map_err(|_| {
        err(
            McpErrorCode::ERR_BACKUP_DECRYPTION_FAILED,
            "failed to decrypt backup; check your passphrase",
        )
    })?;

    Ok(RestorePreviewOutput {
        storage_count: payload.storages.len(),
        has_mcp_settings: payload.mcp_settings.is_some(),
        has_app_settings: payload.app_settings.is_some(),
        created_at: payload.created_at.clone(),
        checksum_valid: payload.verify(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestoreInput {
    pub passphrase: String,
    pub armored: String,
    pub restore_mcp_settings: bool,
    pub restore_app_settings: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestoreOutput {
    pub storages_restored: usize,
    pub mcp_settings_restored: bool,
    pub app_settings_restored: bool,
}

#[tauri::command]
pub fn apply_recovery_restore(
    state: State<'_, AppState>,
    request: ApplyRestoreInput,
) -> Result<ApplyRestoreOutput, McpError> {
    let payload = backup::decrypt_backup(&request.passphrase, &request.armored).map_err(|_| {
        err(
            McpErrorCode::ERR_BACKUP_DECRYPTION_FAILED,
            "failed to decrypt backup; check your passphrase",
        )
    })?;

    let storages: Vec<infimount_mcp::registry::StorageRecord> = payload
        .storages
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    state
        .registry
        .save_legacy_records_secure(storages.clone())
        .map_err(|e| {
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to restore storage registry: {e}"),
            )
        })?;

    let mut mcp_restored = false;
    if request.restore_mcp_settings {
        if let Some(ref settings_val) = payload.mcp_settings {
            if let Ok(settings) = serde_json::from_value::<McpSettings>(settings_val.clone()) {
                if state.settings_store.save_atomic(&settings).is_ok() {
                    mcp_restored = true;
                }
            }
        }
    }

    let mut app_restored = false;
    if request.restore_app_settings {
        if let Some(ref settings_val) = payload.app_settings {
            if let Ok(settings) = serde_json::from_value::<AppSettings>(settings_val.clone()) {
                if state.app_settings_store.save_atomic(&settings).is_ok() {
                    app_restored = true;
                }
            }
        }
    }

    Ok(ApplyRestoreOutput {
        storages_restored: storages.len(),
        mcp_settings_restored: mcp_restored,
        app_settings_restored: app_restored,
    })
}
