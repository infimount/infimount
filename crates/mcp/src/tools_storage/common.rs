use chrono::Utc;
use serde_json::{json, Value};

use crate::errors::{err, err_with_details, McpErrorCode, McpResult};
use crate::policy::McpStoragePolicy;
use crate::registry::{mask_storage_record, validate_storage_name, StorageRecord};

pub(super) fn canonical_backend(backend: &str) -> McpResult<String> {
    let canonical = match backend {
        "local" | "fs" => "local",
        "s3" => "s3",
        "b2" | "backblaze_b2" => "b2",
        "webdav" => "webdav",
        "azure_blob" | "azblob" => "azure_blob",
        "gcs" => "gcs",
        "oss" | "aliyun_oss" => "oss",
        "cos" | "tencent_cos" => "cos",
        "obs" | "huawei_obs" => "obs",
        "sftp" => "sftp",
        "gdrive" | "google_drive" | "google-drive" => "gdrive",
        "onedrive" | "one_drive" | "one-drive" => "onedrive",
        other => {
            return Err(err_with_details(
                McpErrorCode::ERR_BACKEND_UNSUPPORTED,
                format!("unsupported backend '{other}'"),
                json!({ "backend": other }),
            ));
        }
    };

    Ok(canonical.to_string())
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
        let backend = canonical_backend(&self.backend)?;
        ensure_config_object(&self.config)?;
        let now = Utc::now().to_rfc3339();

        Ok(StorageRecord {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            backend,
            config: self.config,
            enabled: self.enabled,
            mcp_exposed: self.mcp_exposed,
            read_only: self.read_only,
            mcp_policy: McpStoragePolicy::default(),
            created_at: self.created_at.unwrap_or_else(|| now.clone()),
            updated_at: self.updated_at.unwrap_or(now),
            ..Default::default()
        })
    }
}
