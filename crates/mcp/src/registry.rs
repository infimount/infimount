use crate::errors::{err, err_with_details, map_core_error, map_io_error, McpErrorCode, McpResult};
use crate::policy::{
    migrate_legacy_policy, normalize_storage_policy, McpStoragePolicy, MCP_POLICY_VERSION,
};
use chrono::Utc;
use fs2::FileExt;
use infimount_core::atomic_file::{atomic_write_file, ensure_parent};
use infimount_core::secrets::{
    discover_secret_field_names, extract_secret_fields, merge_secret_config, strip_secret_fields,
    NativeSecretStore, SecretStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;
use uuid::Uuid;

const REGISTRY_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
pub const STORAGE_RECORD_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub backend: String,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub mcp_exposed: bool,
    pub read_only: bool,
    #[serde(default)]
    pub mcp_policy: McpStoragePolicy,
    #[serde(default)]
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}

fn default_schema_version() -> u32 {
    0
}

fn schema_version_matches_current(version: u32) -> bool {
    version == STORAGE_RECORD_SCHEMA_VERSION
}

impl Default for StorageRecord {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: STORAGE_RECORD_SCHEMA_VERSION,
            id: Uuid::new_v4().to_string(),
            name: String::new(),
            backend: String::new(),
            config: json!({}),
            secret_ref: None,
            secret_fields: Vec::new(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            mcp_policy: McpStoragePolicy::default(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

impl StorageRecord {
    pub fn new(name: String, backend: String, config: Value) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: STORAGE_RECORD_SCHEMA_VERSION,
            id: Uuid::new_v4().to_string(),
            name,
            backend,
            config,
            secret_ref: None,
            secret_fields: Vec::new(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            mcp_policy: McpStoragePolicy::default(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageRegistry {
    path: PathBuf,
    lock_path: PathBuf,
    secret_store: Arc<dyn SecretStore>,
}

impl StorageRegistry {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::with_secret_store(path, Arc::new(NativeSecretStore::new()))
    }

    pub fn with_secret_store(path: Option<PathBuf>, secret_store: Arc<dyn SecretStore>) -> Self {
        let path = path.unwrap_or_else(default_registry_path);
        let lock_path = path.with_extension("lock");
        Self {
            path,
            lock_path,
            secret_store,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        &self.secret_store
    }

    pub fn load_all(&self) -> McpResult<Vec<StorageRecord>> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || self.load_all_unlocked())
    }

    pub fn save_all_atomic(&self, storages: &[StorageRecord]) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            self.save_all_atomic_unlocked(storages)
        })
    }

    pub fn save_all_atomic_if_unchanged(
        &self,
        expected: &[StorageRecord],
        replacement: &[StorageRecord],
    ) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let current = self.load_all_unlocked()?;
            let unchanged = current.len() == expected.len()
                && current.iter().all(|record| {
                    expected.iter().any(|old| {
                        old.id == record.id
                            && old.revision == record.revision
                            && old.updated_at == record.updated_at
                    })
                });
            if !unchanged {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "storage registry changed during transaction; retry the operation",
                ));
            }
            self.save_all_atomic_unlocked(replacement)
        })
    }

    pub fn save_legacy_records_secure(&self, mut storages: Vec<StorageRecord>) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let rollback = self.migrate_secrets_in_batch(&mut storages)?;
            if let Err(error) = self.save_all_atomic_unlocked(&storages) {
                self.rollback_secret_writes(rollback)?;
                return Err(error);
            }
            Ok(())
        })
    }

    pub fn with_locked_mutation<T, F>(&self, mutate: F) -> McpResult<T>
    where
        F: FnOnce(&mut Vec<StorageRecord>) -> McpResult<T>,
    {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let mut storages = self.load_all_unlocked()?;
            let out = mutate(&mut storages)?;
            self.save_all_atomic_unlocked(&storages)?;
            Ok(out)
        })
    }

    pub fn list_exposed_enabled(&self) -> McpResult<Vec<StorageRecord>> {
        let mut storages = self.load_all()?;
        storages.retain(|s| s.enabled && s.mcp_exposed);
        storages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(storages)
    }

    pub fn find_by_name(&self, name: &str) -> McpResult<StorageRecord> {
        let storages = self.load_all()?;
        let Some(storage) = storages.into_iter().find(|s| s.name == name) else {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("Storage '{name}' not found"),
                json!({ "storage_name": name }),
            ));
        };

        if !storage.enabled {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_DISABLED,
                format!("Storage '{name}' is disabled"),
                json!({ "storage_name": name }),
            ));
        }

        if !storage.mcp_exposed {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
                format!("Storage '{name}' is not exposed to MCP"),
                json!({ "storage_name": name }),
            ));
        }

        Ok(storage)
    }

    pub fn resolve_storage(&self, record: &StorageRecord) -> McpResult<ResolvedStorageRecord> {
        let resolved_config = if let Some(ref secret_ref) = record.secret_ref {
            let secret_bundle = self
                .secret_store
                .get_json(secret_ref)
                .map_err(|_| {
                    err_with_details(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "native secret storage is unavailable",
                        json!({ "storage_id": record.id }),
                    )
                })?
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_SECRET_NOT_FOUND,
                        "stored credentials are missing",
                        json!({ "storage_id": record.id }),
                    )
                })?;
            merge_secret_config(&record.config, &secret_bundle)
        } else if record.secret_fields.is_empty() {
            record.config.clone()
        } else {
            return Err(err_with_details(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credential reference is missing",
                json!({ "storage_id": record.id }),
            ));
        };

        Ok(ResolvedStorageRecord {
            record: record.clone(),
            resolved_config,
        })
    }

    fn load_all_unlocked(&self) -> McpResult<Vec<StorageRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let mut storages: Vec<StorageRecord> = serde_json::from_str(&data).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse storage registry",
                json!({ "serde_error": e.to_string(), "path": self.path }),
            )
        })?;

        let needs_policy_migration = storages
            .iter()
            .any(|s| s.mcp_policy.version != MCP_POLICY_VERSION);
        let needs_schema_migration = storages
            .iter()
            .any(|s| !schema_version_matches_current(s.schema_version));
        let schema_secret_names = discover_secret_field_names();
        let needs_secret_migration = storages.iter().any(|storage| {
            infimount_core::secrets::contains_plaintext_secrets(
                &storage.config,
                &schema_secret_names,
            )
        });

        if needs_policy_migration || needs_schema_migration || needs_secret_migration {
            let backup_path = self.create_pre_migration_backup(&data)?;

            let rollback = if needs_secret_migration {
                self.migrate_secrets_in_batch(&mut storages)?
            } else {
                Vec::new()
            };

            let migration_result = (|| -> McpResult<()> {
                for storage in &mut storages {
                    if storage.mcp_policy.version != MCP_POLICY_VERSION {
                        let mut policy = storage.mcp_policy.clone();
                        migrate_legacy_policy(&mut policy)?;
                        storage.mcp_policy = policy;
                    }
                    storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
                }
                self.save_all_atomic_unlocked(&storages)?;
                let persisted = fs::read(&self.path).map_err(|error| {
                    map_io_error(&error, McpErrorCode::ERR_SECRET_MIGRATION_FAILED)
                })?;
                let persisted_records: Vec<StorageRecord> = serde_json::from_slice(&persisted)
                    .map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "failed to verify migrated storage registry",
                        )
                    })?;
                if persisted_records.iter().any(|storage| {
                    infimount_core::secrets::contains_plaintext_secrets(
                        &storage.config,
                        &schema_secret_names,
                    )
                }) {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "plaintext credentials remained after migration",
                    ));
                }
                Ok(())
            })();
            if let Err(error) = migration_result {
                atomic_write_file(&self.path, data.as_bytes(), 0o600).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to restore registry after migration; staged credentials were retained",
                    )
                })?;
                let restored = fs::read(&self.path).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to verify restored registry; staged credentials were retained",
                    )
                })?;
                if restored != data.as_bytes() {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "restored registry verification failed; staged credentials were retained",
                    ));
                }
                self.rollback_secret_writes(rollback)?;
                return Err(error);
            }
            info!("migrated storage registry with backup at {:?}", backup_path);
        }

        Ok(storages)
    }

    fn migrate_secrets_in_batch(
        &self,
        storages: &mut [StorageRecord],
    ) -> McpResult<Vec<(String, Option<Value>)>> {
        let schema_secret_names = discover_secret_field_names();
        let mut rollback = Vec::new();
        for storage in storages.iter_mut() {
            let secret_fields = extract_secret_fields(&storage.config, &schema_secret_names);
            if secret_fields.is_empty() {
                continue;
            }
            let secret_ref = storage
                .secret_ref
                .clone()
                .unwrap_or_else(|| format!("storage/{}", storage.id));
            let previous = match self.secret_store.get_json(&secret_ref) {
                Ok(value) => value,
                Err(_) => {
                    self.rollback_secret_writes(rollback)?;
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to stage credential migration",
                    ));
                }
            };
            let mut bundle = previous.clone().unwrap_or_else(|| json!({}));
            let Some(object) = bundle.as_object_mut() else {
                self.rollback_secret_writes(rollback)?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "stored secret bundle is invalid",
                ));
            };
            let extracted_names = secret_fields
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            object.extend(secret_fields);
            rollback.push((secret_ref.clone(), previous));
            if self.secret_store.put_json(&secret_ref, &bundle).is_err() {
                self.rollback_secret_writes(rollback)?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "failed to migrate credentials to native secret storage",
                ));
            }
            strip_secret_fields(&mut storage.config, &schema_secret_names);
            storage.secret_ref = Some(secret_ref);
            storage.secret_fields = extracted_names;
            storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
            storage.revision = storage.revision.saturating_add(1);
        }
        Ok(rollback)
    }

    fn rollback_secret_writes(&self, rollback: Vec<(String, Option<Value>)>) -> McpResult<()> {
        for (account, previous) in rollback.into_iter().rev() {
            let restored = match previous {
                Some(value) => self.secret_store.put_json(&account, &value),
                None => self.secret_store.delete(&account),
            };
            if restored.is_err() {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "credential rollback failed; manual secret-store cleanup is required",
                ));
            }
        }
        Ok(())
    }

    fn create_pre_migration_backup(&self, original_data: &str) -> McpResult<PathBuf> {
        let backups_dir = self
            .path
            .parent()
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    "registry path has no parent directory",
                    json!({ "path": self.path }),
                )
            })?
            .join("backups");
        infimount_core::atomic_file::create_dir_all(&backups_dir)
            .map_err(|error| map_core_error(&error))?;

        let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f");
        let backup_name = format!("storages.pre-secrets-v2.{}.json", timestamp);
        let backup_path = backups_dir.join(backup_name);

        let payload = original_data.as_bytes();
        atomic_write_file(&backup_path, payload, 0o600).map_err(|e| map_core_error(&e))?;

        Ok(backup_path)
    }

    fn save_all_atomic_unlocked(&self, storages: &[StorageRecord]) -> McpResult<()> {
        ensure_parent(&self.path).map_err(|e| map_core_error(&e))?;

        let mut normalized_storages = storages.to_vec();
        let schema_secret_names = discover_secret_field_names();
        for storage in &mut normalized_storages {
            if infimount_core::secrets::contains_plaintext_secrets(
                &storage.config,
                &schema_secret_names,
            ) {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "refusing to persist plaintext credentials",
                ));
            }
            if storage.mcp_policy.version != MCP_POLICY_VERSION {
                migrate_legacy_policy(&mut storage.mcp_policy)?;
            }
            normalize_storage_policy(&mut storage.mcp_policy)?;
            storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
        }

        let payload = serde_json::to_vec_pretty(&normalized_storages).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize storage registry",
                json!({ "serde_error": e.to_string() }),
            )
        })?;

        atomic_write_file(&self.path, &payload, 0o600).map_err(|e| map_core_error(&e))
    }

    fn with_file_lock<T>(
        &self,
        timeout: Duration,
        f: impl FnOnce() -> McpResult<T>,
    ) -> McpResult<T> {
        ensure_parent(&self.lock_path).map_err(|e| map_core_error(&e))?;

        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        let start = Instant::now();
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) if start.elapsed() >= timeout => {
                    return Err(err(
                        McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                        "timed out acquiring storage registry lock",
                    ));
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }

        let result = f();
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

pub struct ResolvedStorageRecord {
    pub record: StorageRecord,
    pub resolved_config: serde_json::Value,
}

pub fn retry_pending_secret_cleanup(secret_store: &dyn SecretStore) -> McpResult<()> {
    let path = default_config_dir().join("secret-cleanup.json");
    if !path.exists() {
        return Ok(());
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))
        .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
    let start = Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if start.elapsed() >= Duration::from_secs(2) => {
                return Err(err(
                    McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                    "timed out acquiring secret cleanup journal lock",
                ));
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
    let document: Value = serde_json::from_slice(
        &fs::read(&path).map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "secret cleanup journal is invalid",
        )
    })?;
    let active_secret_refs = if default_registry_path().exists() {
        serde_json::from_slice::<Vec<StorageRecord>>(
            &fs::read(default_registry_path())
                .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?,
        )
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "storage registry is invalid"))?
        .into_iter()
        .filter_map(|record| record.secret_ref)
        .collect::<std::collections::HashSet<_>>()
    } else {
        std::collections::HashSet::new()
    };
    let remaining = document
        .get("pending")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            item.get("account")
                .and_then(Value::as_str)
                .is_some_and(|account| {
                    if active_secret_refs.contains(account) {
                        false
                    } else {
                        secret_store.delete(account).is_err()
                    }
                })
        })
        .collect::<Vec<_>>();
    if remaining.is_empty() {
        fs::remove_file(&path).map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
    } else {
        let payload =
            serde_json::to_vec_pretty(&json!({ "pending": remaining })).map_err(|_| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    "failed to update cleanup journal",
                )
            })?;
        atomic_write_file(&path, &payload, 0o600).map_err(|error| map_core_error(&error))?;
    }
    Ok(())
}

pub fn validate_storage_name(raw: &str) -> McpResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must not be empty",
        ));
    }
    if name == "/" {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name '/' is invalid",
        ));
    }
    if name.contains('/') {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must not contain '/'",
        ));
    }
    if name.chars().count() > 64 {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must be at most 64 characters",
        ));
    }

    Ok(name.to_string())
}

pub fn ensure_unique_name(
    storages: &[StorageRecord],
    name: &str,
    except_id: Option<&str>,
) -> McpResult<()> {
    let conflict = storages.iter().any(|s| {
        if let Some(except_id) = except_id {
            if s.id == except_id {
                return false;
            }
        }
        s.name == name
    });

    if conflict {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_NAME_CONFLICT,
            format!("Storage name '{name}' already exists"),
            json!({ "name": name }),
        ));
    }

    Ok(())
}

pub fn mask_storage_record(storage: &StorageRecord) -> StorageRecord {
    fn insert_mask(config: &mut Value, path: &str) {
        let Some(root) = config.as_object_mut() else {
            return;
        };
        let mut current = root;
        let mut parts = path.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                current.insert(part.to_string(), Value::String("********".to_string()));
                return;
            }
            let entry = current
                .entry(part.to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            current = entry.as_object_mut().expect("mask path object");
        }
    }
    let mut masked = storage.clone();
    masked.config = mask_secrets_in_value(&masked.config);
    for field in &masked.secret_fields {
        insert_mask(&mut masked.config, field);
    }
    masked
}

pub fn mask_secrets_in_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if is_secret_key(key) {
                    out.insert(key.clone(), Value::String("********".to_string()));
                } else {
                    out.insert(key.clone(), mask_secrets_in_value(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(mask_secrets_in_value).collect()),
        _ => value.clone(),
    }
}

pub fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "token",
        "access_key",
        "accesskey",
        "secret_key",
        "client_secret",
        "session_token",
        "keyid",
        "applicationkey",
        "application_key",
        "credential",
        "privatekey",
        "private_key",
        "privatekeypath",
        "private_key_path",
        "keypath",
        "key_path",
        "key",
        "serviceaccountjson",
        "service_account_json",
        "codeverifier",
        "code_verifier",
        "devicecode",
        "device_code",
        "authcode",
        "auth_code",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub fn default_config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        return PathBuf::from(base).join("infimount");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".infimount")
    }
}

pub fn default_registry_path() -> PathBuf {
    default_config_dir().join("storages.json")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;

    use super::*;
    use crate::policy::McpAccessMode;

    #[test]
    fn schema_v2_plaintext_is_migrated_to_memory_secret_store() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("storages.json");
        let secret_store = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(Some(path.clone()), secret_store.clone());
        let mut record = StorageRecord::new(
            "S3".to_string(),
            "s3".to_string(),
            json!({
                "bucket": "example",
                "nested": { "secretAccessKey": "seeded-secret-value" }
            }),
        );
        record.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
        fs::write(&path, serde_json::to_vec_pretty(&vec![record]).unwrap()).unwrap();

        let loaded = registry.load_all().expect("migrate registry");
        assert!(loaded[0]
            .config
            .pointer("/nested/secretAccessKey")
            .is_none());
        assert_eq!(
            secret_store
                .get_json(&format!("storage/{}", loaded[0].id))
                .unwrap()
                .unwrap()["nested.secretAccessKey"],
            "seeded-secret-value"
        );
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("seeded-secret-value"));
    }

    #[test]
    fn unavailable_secret_store_preserves_plaintext_registry_bytes() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("storages.json");
        let registry = StorageRegistry::with_secret_store(
            Some(path.clone()),
            std::sync::Arc::new(infimount_core::secrets::UnavailableSecretStore::new(
                "locked",
            )),
        );
        let record = StorageRecord::new(
            "S3".to_string(),
            "s3".to_string(),
            json!({ "secretAccessKey": "seeded-secret-value" }),
        );
        let original = serde_json::to_vec_pretty(&vec![record]).unwrap();
        fs::write(&path, &original).unwrap();
        let error = registry.load_all().expect_err("migration should fail");
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_MIGRATION_FAILED);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn storage_name_rules() {
        assert!(validate_storage_name("  photos ").is_ok());
        assert!(validate_storage_name("").is_err());
        assert!(validate_storage_name("/").is_err());
        assert!(validate_storage_name("a/b").is_err());
        assert!(validate_storage_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn secret_masking_recursive() {
        let input = json!({
            "token": "abc",
            "accessKeyId": "access-key-id",
            "applicationKey": "application-key",
            "application_key": "application-key-snake",
            "credential": "service-account-json",
            "serviceAccountJson": "service-account-json",
            "service_account_json": "service-account-json",
            "privateKeyPath": "/home/alice/.ssh/id_ed25519",
            "keyPath": "/home/alice/.ssh/id_rsa",
            "key": "/home/alice/.ssh/id_ecdsa",
            "accessToken": "oauth-access-token",
            "refreshToken": "oauth-refresh-token",
            "clientSecret": "oauth-client-secret",
            "codeVerifier": "pkce-verifier",
            "deviceCode": "oauth-device-code",
            "nested": {
                "client_secret": "x",
                "secretId": "secret-id",
                "safe": "ok"
            }
        });

        let masked = mask_secrets_in_value(&input);
        assert_eq!(masked["token"], "********");
        assert_eq!(masked["accessKeyId"], "********");
        assert_eq!(masked["applicationKey"], "********");
        assert_eq!(masked["application_key"], "********");
        assert_eq!(masked["credential"], "********");
        assert_eq!(masked["serviceAccountJson"], "********");
        assert_eq!(masked["service_account_json"], "********");
        assert_eq!(masked["privateKeyPath"], "********");
        assert_eq!(masked["keyPath"], "********");
        assert_eq!(masked["key"], "********");
        assert_eq!(masked["accessToken"], "********");
        assert_eq!(masked["refreshToken"], "********");
        assert_eq!(masked["clientSecret"], "********");
        assert_eq!(masked["codeVerifier"], "********");
        assert_eq!(masked["deviceCode"], "********");
        assert_eq!(masked["nested"]["client_secret"], "********");
        assert_eq!(masked["nested"]["secretId"], "********");
        assert_eq!(masked["nested"]["safe"], "ok");
    }

    /// Creates a minimal v1 policy storage JSON for migration testing
    fn v1_policy_storage_json(name: &str, default_access: &str) -> String {
        format!(
            r#"{{
    "schema_version": 1,
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "version": 1,
        "default_access": "{default_access}",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }},
        "allowed_paths": ["projects"]
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#
        )
    }

    /// Creates a schema v0 (versionless) storage JSON with versionless policy
    fn v0_schema_v0_policy_storage_json(name: &str) -> String {
        format!(
            r#"{{
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "default_access": "read_only",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }}
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#
        )
    }

    fn write_registry(dir: &TempDir, data: &str) {
        fs::write(dir.path().join("storages.json"), data).expect("write registry");
    }

    fn load_registry(dir: &TempDir) -> Vec<StorageRecord> {
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        registry.load_all().expect("load registry")
    }

    #[test]
    fn migration_v1_policy_to_v2() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("photos", "read_write")),
        );

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        let s = &storages[0];
        assert_eq!(s.name, "photos");
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        // v1 policy with default_access=read_write and allowed_paths=["projects"]
        // should migrate to v2 with default_access=None and a ReadWrite rule for "projects"
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadWrite);
        assert!(s.mcp_policy.allowed_paths.is_empty());
    }

    #[test]
    fn migration_v0_schema_and_v0_policy() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v0_schema_v0_policy_storage_json("legacy")),
        );

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        let s = &storages[0];
        assert_eq!(s.name, "legacy");
        // versionless (v0) policy gets version=0 after deserialization, then migration upgrades it
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        // versionless policy has default_access=read_only, no rules
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn migration_persists_denied_paths_from_v1() {
        let dir = TempDir::new().expect("temp dir");
        let json = v1_policy_storage_json("secure", "read_only")
            .replace(r#""denied_paths": []"#, r#""denied_paths": ["secrets"]"#);
        write_registry(&dir, &format!("[\n{json}\n]"));

        let storages = load_registry(&dir);
        let s = &storages[0];
        assert_eq!(s.mcp_policy.denied_paths, vec!["secrets"]);
        // allowed_paths was migrated to rules
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
        // Since default_access was read_only, migrated rule gets ReadOnly
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn migration_backup_is_byte_for_byte() {
        let dir = TempDir::new().expect("temp dir");
        let original_json = format!("[\n{}\n]", v1_policy_storage_json("photos", "read_only"));
        write_registry(&dir, &original_json);

        let _storages = load_registry(&dir);

        // Check that backup was written to backups/
        let backups_dir = dir.path().join("backups");
        assert!(backups_dir.exists(), "backups directory should exist");
        let backup_files: Vec<_> = fs::read_dir(&backups_dir)
            .expect("read backups dir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(!backup_files.is_empty(), "backup files should exist");

        // Verify backup content matches original byte-for-byte
        let backup_content = fs::read_to_string(backup_files[0].path()).expect("read backup");
        assert_eq!(
            backup_content, original_json,
            "backup must match original byte-for-byte"
        );
    }

    #[test]
    fn migration_persistence_after_reload() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("persist", "read_write")),
        );

        // Load once to trigger migration
        let _first = load_registry(&dir);

        // Load again - should NOT re-migrate
        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        assert_eq!(storages[0].mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(storages[0].schema_version, STORAGE_RECORD_SCHEMA_VERSION);
    }

    #[test]
    fn atomic_write_file_creates_with_0600_perms() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.json");
        let payload = b"{\"key\":\"value\"}";

        atomic_write_file(&path, payload, 0o600).expect("atomic write");

        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&path).expect("metadata");
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "file permissions should be 0600");
        }

        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "{\"key\":\"value\"}");
    }

    #[test]
    fn atomic_write_file_persists_content() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("persist.json");
        let payload = b"persistent content";

        atomic_write_file(&path, payload, 0o600).expect("atomic write");
        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "persistent content");

        // Overwrite
        let payload2 = b"updated content";
        atomic_write_file(&path, payload2, 0o600).expect("atomic overwrite");
        let content2 = fs::read_to_string(&path).expect("read");
        assert_eq!(content2, "updated content");
    }

    #[test]
    fn backup_failure_preserves_original_registry() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("original", "read_only")),
        );

        // A file at the backup directory path fails deterministically on every platform.
        fs::write(dir.path().join("backups"), b"not a directory").expect("create backup blocker");

        // Loading should fail because backup can't be written
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        let result = registry.load_all();
        assert!(
            result.is_err(),
            "load should fail when backup cannot be written"
        );

        // Original data should still be intact
        let original_content =
            fs::read_to_string(dir.path().join("storages.json")).expect("read original");
        assert!(
            original_content.contains("original"),
            "original registry should be preserved"
        );
    }

    #[test]
    fn mixed_schema_v1_policy_v2_not_migrated() {
        // A registry that already has schema v1 and policy v2 should not be migrated
        let dir = TempDir::new().expect("temp dir");
        let json = v1_policy_storage_json("already-v2", "read_write")
            .replace(r#""version": 1"#, r#""version": 2"#);
        write_registry(&dir, &format!("[\n{json}\n]"));

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        assert_eq!(storages[0].mcp_policy.version, MCP_POLICY_VERSION);
        // Should not have re-migrated
        assert!(storages[0].mcp_policy.rules.is_empty());
        assert_eq!(
            storages[0].mcp_policy.default_access,
            McpAccessMode::ReadWrite
        );
    }

    // ── Migration Matrix Tests ──────────────────────────────────────────

    /// Generates a storage JSON with explicit schema_version and policy version
    fn matrix_storage_json(
        name: &str,
        schema_version: u32,
        policy_version: u32,
        policy_default_access: &str,
        allowed_paths: &[&str],
    ) -> String {
        let allowed = if allowed_paths.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = allowed_paths.iter().map(|p| format!("\"{p}\"")).collect();
            format!("[{}]", items.join(", "))
        };
        format!(
            r#"{{
    "schema_version": {sv},
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "version": {pv},
        "default_access": "{pda}",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }},
        "allowed_paths": {allowed}
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#,
            sv = schema_version,
            name = name,
            pv = policy_version,
            pda = policy_default_access,
            allowed = allowed
        )
    }

    fn load_single(dir: &TempDir) -> StorageRecord {
        let storages = load_registry(dir);
        assert_eq!(storages.len(), 1, "expected exactly 1 storage");
        storages.into_iter().next().unwrap()
    }

    #[test]
    fn matrix_schema_v0_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s0p0", 0, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v0_policy_v1() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s0p1", 0, 1, "read_write", &["data"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "data");
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadWrite);
    }

    #[test]
    fn matrix_schema_v1_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p0", 1, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v1_policy_v1_allowed_to_rules_migration() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p1", 1, 1, "read_only", &["docs", "assets"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 2);
        assert_eq!(s.mcp_policy.rules[0].prefix, "docs");
        assert_eq!(s.mcp_policy.rules[1].prefix, "assets");
        // v1 default_access=read_only + allowed_paths -> migrated to ReadOnly rules
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v1_policy_v2_no_op() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p2", 1, 2, "read_write", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        // Policy v2 with no allowed_paths, default_access=read_write should stay
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadWrite);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn matrix_schema_v2_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p0", 2, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v2_policy_v1() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p1", 2, 1, "read_write", &["projects"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
    }

    #[test]
    fn matrix_schema_v2_policy_v2_no_op() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p2", 2, 2, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn matrix_multiple_storages_mixed_versions() {
        let dir = TempDir::new().expect("temp dir");
        let s0 = matrix_storage_json("legacy-v0", 0, 0, "read_only", &[]);
        let s1 = matrix_storage_json("v1-policy", 1, 1, "read_write", &["data"]);
        let s2 = matrix_storage_json("current", 2, 2, "read_only", &[]);
        write_registry(&dir, &format!("[\n{s0},\n{s1},\n{s2}\n]"));

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 3);

        let legacy = storages.iter().find(|s| s.name == "legacy-v0").unwrap();
        assert_eq!(legacy.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(legacy.mcp_policy.version, MCP_POLICY_VERSION);

        let migrated = storages.iter().find(|s| s.name == "v1-policy").unwrap();
        assert_eq!(migrated.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(migrated.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(migrated.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(migrated.mcp_policy.rules.len(), 1);

        let current = storages.iter().find(|s| s.name == "current").unwrap();
        assert_eq!(current.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(current.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(current.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }
}
