use chrono::Utc;
use serde_json::{json, Value};

use crate::errors::{err, err_with_details, McpErrorCode, McpResult};
use crate::registry::{mask_storage_record, validate_storage_name, StorageRecord};

pub(super) fn ensure_backend_supported(backend: &str) -> McpResult<()> {
    if matches!(
        backend,
        "local" | "fs" | "s3" | "webdav" | "azure_blob" | "azblob" | "gcs"
    ) {
        return Ok(());
    }

    Err(err_with_details(
        McpErrorCode::ERR_BACKEND_UNSUPPORTED,
        format!("unsupported backend '{backend}'"),
        json!({ "backend": backend }),
    ))
}

pub(super) fn ensure_config_object(config: &Value) -> McpResult<()> {
    if config.is_object() {
        Ok(())
    } else {
        Err(err(
            McpErrorCode::ERR_INTERNAL,
            "storage config must be a JSON object",
        ))
    }
}

pub(super) fn masked(storage: &StorageRecord) -> StorageRecord {
    mask_storage_record(storage)
}

pub(super) fn next_renamed_name(existing: &[StorageRecord], base_name: &str) -> String {
    let mut idx = 2_u32;
    loop {
        let candidate = format!("{base_name} ({idx})");
        if existing.iter().all(|storage| storage.name != candidate) {
            return candidate;
        }
        idx += 1;
    }
}

#[derive(Debug, Clone)]
pub(super) struct ImportedStorage {
    pub name: String,
    pub backend: String,
    pub config: Value,
    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
    pub id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl ImportedStorage {
    pub(super) fn into_record(self) -> McpResult<StorageRecord> {
        let name = validate_storage_name(&self.name)?;
        ensure_backend_supported(&self.backend)?;
        ensure_config_object(&self.config)?;
        let now = Utc::now().to_rfc3339();

        Ok(StorageRecord {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            backend: self.backend,
            config: self.config,
            enabled: self.enabled,
            mcp_exposed: self.mcp_exposed,
            read_only: self.read_only,
            created_at: self.created_at.unwrap_or_else(|| now.clone()),
            updated_at: self.updated_at.unwrap_or(now),
        })
    }
}
