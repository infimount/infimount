use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::{err, err_with_details, McpErrorCode, McpResult};
use crate::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use crate::tools_fs::FsToolsContext;

use super::common::{canonical_backend, ensure_config_object, masked};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub async fn edit_storage(
    ctx: &FsToolsContext,
    input: EditStorageInput,
) -> McpResult<EditStorageOutput> {
    let canonical_patch_backend = input
        .patch
        .backend
        .as_deref()
        .map(canonical_backend)
        .transpose()?;
    if let Some(ref config) = input.patch.config {
        ensure_config_object(config)?;
    }

    let mut previous: Option<Value> = None;
    let mut changed_secret = false;
    let mut previous_account = String::new();
    let mut staged_account = String::new();
    let result = ctx.registry.with_locked_mutation(|storages| {
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
        previous_account = storage
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{}", storage.id));
        staged_account = format!(
            "storage/{}/revision/{}/{}",
            storage.id,
            storage.revision.saturating_add(1),
            Uuid::new_v4()
        );
        previous = ctx
            .registry
            .secret_store()
            .get_json(&previous_account)
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage credentials",
                )
            })?;
        if previous.is_none() && (storage.secret_ref.is_some() || !storage.secret_fields.is_empty())
        {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        let mut staged_bundle = previous.clone().unwrap_or_else(|| serde_json::json!({}));
        let mut public_config = input.patch.config.clone();
        if let Some(config) = public_config.as_mut() {
            let secret_names = infimount_core::secrets::discover_secret_field_names();
            let extracted = infimount_core::secrets::extract_secret_fields(config, &secret_names);
            let bundle = staged_bundle.as_object_mut().ok_or_else(|| {
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "stored secret bundle is invalid",
                )
            })?;
            bundle.extend(extracted);
            infimount_core::secrets::strip_secret_fields(config, &secret_names);
        }
        let has_secrets = staged_bundle
            .as_object()
            .is_some_and(|object| !object.is_empty());

        if let Some(ref new_name) = input.patch.new_name {
            let normalized_name = validate_storage_name(new_name)?;
            ensure_unique_name(storages, &normalized_name, Some(storage.id.as_str()))?;
            storage.name = normalized_name;
        }
        if let Some(ref backend) = canonical_patch_backend {
            storage.backend = backend.clone();
        }
        if let Some(ref config) = public_config {
            if has_secrets
                && ctx
                    .registry
                    .secret_store()
                    .put_json(&staged_account, &staged_bundle)
                    .is_err()
            {
                if ctx.registry.secret_store().delete(&staged_account).is_err() {
                    super::import_config::append_cleanup_journal(&staged_account)?;
                }
                return Err(err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage updated credentials",
                ));
            }
            changed_secret = has_secrets;
            storage.config = config.clone();
            storage.secret_ref = has_secrets.then(|| staged_account.clone());
            storage.secret_fields = staged_bundle
                .as_object()
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default();
            storage.revision = storage.revision.saturating_add(1);
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
    });
    let updated = match result {
        Ok(updated) => updated,
        Err(error) => {
            if changed_secret && ctx.registry.secret_store().delete(&staged_account).is_err() {
                super::import_config::append_cleanup_journal(&staged_account).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "credential rollback failed and could not be journaled",
                    )
                })?;
            }
            return Err(error);
        }
    };

    let mut warning = None;
    if previous.is_some()
        && updated.secret_ref.as_deref() != Some(previous_account.as_str())
        && ctx
            .registry
            .secret_store()
            .delete(&previous_account)
            .is_err()
    {
        warning = Some(
            if super::import_config::append_cleanup_journal(&previous_account).is_ok() {
                "Previous credential cleanup is pending and will be retried."
            } else {
                "Previous credential cleanup failed and could not be journaled; remove the old native secret-store entry manually."
            }
            .to_string(),
        );
    }

    Ok(EditStorageOutput {
        storage: masked(&updated),
        warning,
    })
}
