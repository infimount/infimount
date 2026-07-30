use std::collections::HashMap;

use infimount_core::backup::{self, BackupPayload};
use infimount_core::secrets::SecretStore;
use infimount_core::workspaces::WorkspaceRegistry;
use infimount_mcp::errors::{err, McpErrorCode, McpError};
use infimount_mcp::registry::StorageRecord;
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
    pub has_native_secrets: bool,
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

    let workspaces = state
        .workspaces
        .load_all()
        .ok()
        .map(|ws| serde_json::to_value(ws).unwrap_or_default());

    let secret_names = collect_secret_account_names().unwrap_or_default();
    let mut secrets = HashMap::new();
    for name in &secret_names {
        if let Ok(Some(value)) = state.secret_store.get_json(name) {
            if let Ok(serialized) = serde_json::to_string(&value) {
                secrets.insert(name.clone(), serialized);
            }
        }
    }

    let has_native_secrets = !secrets.is_empty();

    let payload =
        BackupPayload::new(storage_values, mcp_settings, app_settings, workspaces, secrets)
            .map_err(|e| {
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
        has_native_secrets,
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
    pub has_workspaces: bool,
    pub has_secrets: bool,
    pub created_at: String,
    pub checksum_valid: bool,
    pub unsupported_version: bool,
}

#[tauri::command]
pub fn preview_recovery_restore(
    request: RestorePreviewInput,
) -> Result<RestorePreviewOutput, McpError> {
    let payload = match backup::decrypt_backup(&request.passphrase, &request.armored) {
        Ok(p) => p,
        Err(e) => {
            return Ok(RestorePreviewOutput {
                storage_count: 0,
                has_mcp_settings: false,
                has_app_settings: false,
                has_workspaces: false,
                has_secrets: false,
                created_at: String::new(),
                checksum_valid: false,
                unsupported_version: matches!(
                    e,
                    backup::BackupError::Serialization(_)
                ),
            });
        }
    };

    Ok(RestorePreviewOutput {
        storage_count: payload.storages.len(),
        has_mcp_settings: payload.mcp_settings.is_some(),
        has_app_settings: payload.app_settings.is_some(),
        has_workspaces: payload.workspaces.is_some(),
        has_secrets: !payload.secrets.is_empty(),
        created_at: payload.created_at.clone(),
        checksum_valid: payload.verify(),
        unsupported_version: false,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestoreInput {
    pub passphrase: String,
    pub armored: String,
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

    let storages: Vec<StorageRecord> = payload
        .storages
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    if storages.len() != payload.storages.len() {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "one or more storage records in the backup are malformed",
        ));
    }

    let mut mcp_settings: Option<McpSettings> = None;
    if request.restore_mcp_settings {
        mcp_settings = payload
            .mcp_settings
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());
    }

    let mut app_settings: Option<AppSettings> = None;
    if request.restore_app_settings {
        app_settings = payload
            .app_settings
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());
    }

    let mut workspaces_opt: Option<serde_json::Value> = None;
    if request.restore_workspaces {
        workspaces_opt = payload.workspaces.clone();
    }

    let mut secrets_to_restore: HashMap<String, serde_json::Value> = HashMap::new();
    if request.restore_secrets {
        for (name, serialized) in &payload.secrets {
            if let Ok(value) = serde_json::from_str(serialized) {
                secrets_to_restore.insert(name.clone(), value);
            } else {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    format!("malformed secret value for '{name}'"),
                ));
            }
        }
    }

    let legacy_registry = state.registry.load_all().ok();
    let legacy_mcp = state.settings_store.load().ok();
    let legacy_app = state.app_settings_store.load().ok();
    let legacy_workspaces = state.workspaces.load_all().ok();
    let legacy_secrets = collect_legacy_secrets(state.secret_store.as_ref(), &payload.secrets).ok();

    let rollback = || -> Result<(), McpError> {
        if let Some(ref records) = legacy_registry {
            let _ = state.registry.save_legacy_records_secure(records.clone());
        }
        if let Some(ref settings) = legacy_mcp {
            let _ = state.settings_store.save_atomic(settings);
        }
        if let Some(ref settings) = legacy_app {
            let _ = state.app_settings_store.save_atomic(settings);
        }
        if let Some(ref workspaces) = legacy_workspaces {
            let _ = state.workspaces.save(workspaces);
        }
        if let Some(ref secrets_map) = legacy_secrets {
            for (name, value) in secrets_map {
                let _ = state.secret_store.put_json(name, value);
            }
        }
        Ok(())
    };

    state
        .registry
        .save_legacy_records_secure(storages.clone())
        .map_err(|e| {
            let _ = rollback();
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to restore storage registry: {e}"),
            )
        })?;

    let mut mcp_restored = false;
    if let Some(ref settings) = mcp_settings {
        mcp_restored = state.settings_store.save_atomic(settings).is_ok();
        if !mcp_restored {
            let _ = rollback();
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "failed to restore MCP settings",
            ));
        }
    }

    let mut app_restored = false;
    if let Some(ref settings) = app_settings {
        app_restored = state.app_settings_store.save_atomic(settings).is_ok();
        if !app_restored {
            let _ = rollback();
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "failed to restore app settings",
            ));
        }
    }

    let mut workspaces_restored = false;
    if let Some(ref ws_value) = workspaces_opt {
        if let Ok(records) =
            serde_json::from_value::<Vec<infimount_core::workspaces::WorkspaceRecord>>(
                ws_value.clone(),
            )
        {
            workspaces_restored = state.workspaces.save(&records).is_ok();
            if !workspaces_restored {
                let _ = rollback();
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "failed to restore workspaces",
                ));
            }
        }
    }

    let mut secrets_restored = 0usize;
    for (name, value) in &secrets_to_restore {
        if state.secret_store.put_json(name, value).is_ok() {
            secrets_restored += 1;
        } else {
            let _ = rollback();
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to restore secret '{name}'"),
            ));
        }
    }

    Ok(ApplyRestoreOutput {
        storages_restored: storages.len(),
        mcp_settings_restored: mcp_restored,
        app_settings_restored: app_restored,
        workspaces_restored,
        secrets_restored,
    })
}

fn collect_secret_account_names() -> Result<Vec<String>, McpError> {
    let known = infimount_core::secrets::discover_secret_field_names();
    let mut names: Vec<String> = known
        .into_iter()
        .filter(|n| {
            !n.is_empty()
                && n != "accessKeyId"
                && n != "secretAccessKey"
        })
        .collect();
    names.push("com.infimount.mcp-auth-token".into());
    names.sort();
    names.dedup();
    Ok(names)
}

fn collect_legacy_secrets(
    secret_store: &dyn SecretStore,
    backup_secrets: &HashMap<String, String>,
) -> Result<HashMap<String, serde_json::Value>, McpError> {
    let mut legacy = HashMap::new();
    for name in backup_secrets.keys() {
        if let Ok(Some(value)) = secret_store.get_json(name) {
            legacy.insert(name.clone(), value);
        }
    }
    Ok(legacy)
}
