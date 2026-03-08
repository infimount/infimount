use crate::errors::{err, err_with_details, map_io_error, McpErrorCode, McpResult};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const REGISTRY_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRecord {
    pub id: String,
    pub name: String,
    pub backend: String,
    pub config: Value,
    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl StorageRecord {
    pub fn new(name: String, backend: String, config: Value) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            backend,
            config,
            enabled: true,
            mcp_exposed: true,
            read_only: false,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageRegistry {
    path: PathBuf,
    lock_path: PathBuf,
}

impl StorageRegistry {
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_registry_path);
        let lock_path = path.with_extension("lock");
        Self { path, lock_path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_all(&self) -> McpResult<Vec<StorageRecord>> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || self.load_all_unlocked())
    }

    pub fn save_all_atomic(&self, storages: &[StorageRecord]) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            self.save_all_atomic_unlocked(storages)
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

    fn load_all_unlocked(&self) -> McpResult<Vec<StorageRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let storages: Vec<StorageRecord> = serde_json::from_str(&data).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse storage registry",
                json!({ "serde_error": e.to_string(), "path": self.path }),
            )
        })?;
        Ok(storages)
    }

    fn save_all_atomic_unlocked(&self, storages: &[StorageRecord]) -> McpResult<()> {
        ensure_parent(&self.path)?;

        let parent = self.path.parent().ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "registry path has no parent directory",
                json!({ "path": self.path }),
            )
        })?;

        let tmp_name = format!(
            ".storages.json.tmp.{}.{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let tmp_path = parent.join(tmp_name);

        let payload = serde_json::to_vec_pretty(storages).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize storage registry",
                json!({ "serde_error": e.to_string() }),
            )
        })?;

        fs::write(&tmp_path, payload).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        fs::rename(&tmp_path, &self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        Ok(())
    }

    fn with_file_lock<T>(
        &self,
        timeout: Duration,
        f: impl FnOnce() -> McpResult<T>,
    ) -> McpResult<T> {
        ensure_parent(&self.lock_path)?;

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
        let _ = lock_file.unlock();
        result
    }
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
    let mut masked = storage.clone();
    masked.config = mask_secrets_in_value(&masked.config);
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
        "secret_key",
        "client_secret",
        "session_token",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub fn default_registry_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        return PathBuf::from(base).join("infimount").join("storages.json");
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.trim().is_empty() {
                return PathBuf::from(xdg).join("infimount").join("storages.json");
            }
        }

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("infimount")
            .join("storages.json")
    }
}

fn ensure_parent(path: &Path) -> McpResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "nested": {
                "client_secret": "x",
                "safe": "ok"
            }
        });

        let masked = mask_secrets_in_value(&input);
        assert_eq!(masked["token"], "********");
        assert_eq!(masked["nested"]["client_secret"], "********");
        assert_eq!(masked["nested"]["safe"], "ok");
    }
}
