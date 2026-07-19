use crate::errors::{err, err_with_details, map_io_error, McpErrorCode, McpResult};
use crate::registry::default_config_dir;
use crate::server::{all_tool_names, default_enabled_tool_names};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::SystemTime;
use std::time::{Duration, Instant};

const SETTINGS_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_HTTP_BIND_ADDRESS: &str = "127.0.0.1";
pub const DEFAULT_HTTP_PORT: u16 = 7331;

pub const SECURITY_BASELINE_VERSION: u32 = 2;

static PRE_MIGRATION_PREFIX: &str = "mcp_settings.pre-v0.8.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub enabled: bool,
    pub transport: McpTransport,
    pub bind_address: String,
    pub port: u16,
    #[serde(default = "default_enabled_tool_names")]
    pub enabled_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub security_baseline_version: u32,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: McpTransport::Stdio,
            bind_address: DEFAULT_HTTP_BIND_ADDRESS.to_string(),
            port: DEFAULT_HTTP_PORT,
            enabled_tools: default_enabled_tool_names(),
            auth_token: None,
            security_baseline_version: SECURITY_BASELINE_VERSION,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpSettingsStore {
    path: PathBuf,
    lock_path: PathBuf,
}

impl McpSettingsStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_settings_path);
        let lock_path = path.with_extension("lock");
        Self { path, lock_path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> McpResult<McpSettings> {
        self.with_file_lock(SETTINGS_LOCK_TIMEOUT, || self.load_unlocked())
    }

    pub fn save_atomic(&self, settings: &McpSettings) -> McpResult<()> {
        self.with_file_lock(SETTINGS_LOCK_TIMEOUT, || {
            self.save_atomic_unlocked(settings)
        })
    }

    fn load_unlocked(&self) -> McpResult<McpSettings> {
        if !self.path.exists() {
            return Ok(McpSettings::default());
        }

        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let settings: McpSettings = serde_json::from_str(&data).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse MCP settings",
                serde_json::json!({ "serde_error": e.to_string(), "path": self.path }),
            )
        })?;
        let requires_migration = settings.security_baseline_version < SECURITY_BASELINE_VERSION;
        let normalized = normalize_settings(settings);
        if requires_migration {
            self.create_pre_migration_backup(data.as_bytes())?;
            self.save_atomic_unlocked(&normalized)?;
        }
        Ok(normalized)
    }

    fn save_atomic_unlocked(&self, settings: &McpSettings) -> McpResult<()> {
        ensure_parent(&self.path)?;

        let normalized = normalize_settings(settings.clone());
        let payload = serde_json::to_vec_pretty(&normalized).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize MCP settings",
                serde_json::json!({ "serde_error": e.to_string() }),
            )
        })?;

        write_sensitive_atomic(&self.path, &payload)
    }

    fn create_pre_migration_backup(&self, original: &[u8]) -> McpResult<PathBuf> {
        let parent = self.path.parent().ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "settings path has no parent directory",
                serde_json::json!({ "path": self.path }),
            )
        })?;
        let backups_dir = parent.join("backups");
        fs::create_dir_all(&backups_dir)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let backup_path = backups_dir.join(format!("{PRE_MIGRATION_PREFIX}{timestamp}.json"));
        write_sensitive_atomic(&backup_path, original)?;
        Ok(backup_path)
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
                        "timed out acquiring MCP settings lock",
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

pub fn default_settings_path() -> PathBuf {
    default_config_dir().join("mcp_settings.json")
}

fn write_sensitive_atomic(path: &Path, payload: &[u8]) -> McpResult<()> {
    ensure_parent(path)?;
    let parent = path.parent().ok_or_else(|| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "settings path has no parent directory",
            serde_json::json!({ "path": path }),
        )
    })?;
    let tmp_path = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("mcp_settings"),
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default()
    ));

    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp_path)?;
        file.write_all(payload)?;
        file.sync_all()?;
        replace_file(&tmp_path, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_file(&tmp_path);
        return Err(map_io_error(&error, McpErrorCode::ERR_INTERNAL));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
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

fn normalize_settings(mut settings: McpSettings) -> McpSettings {
    settings = migrate_security_baseline(settings);
    settings.enabled_tools = sanitize_enabled_tools(settings.enabled_tools);
    settings.auth_token = normalize_auth_token(settings.auth_token);
    settings
}

pub fn normalize_auth_token(value: Option<String>) -> Option<String> {
    value
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn sanitize_enabled_tools(enabled_tools: Vec<String>) -> Vec<String> {
    let allowed = all_tool_names().into_iter().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    enabled_tools
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && allowed.contains(value) && seen.insert(value.clone()))
        .collect()
}

fn migrate_security_baseline(mut settings: McpSettings) -> McpSettings {
    if settings.security_baseline_version >= SECURITY_BASELINE_VERSION {
        return settings;
    }

    let previous = settings.enabled_tools.iter().collect::<HashSet<_>>();
    let intersected = default_enabled_tool_names()
        .into_iter()
        .filter(|name| previous.contains(name))
        .collect::<Vec<_>>();
    settings.enabled_tools = if intersected.is_empty() {
        default_enabled_tool_names()
    } else {
        intersected
    };
    settings.security_baseline_version = SECURITY_BASELINE_VERSION;
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safe_tools() -> Vec<String> {
        default_enabled_tool_names()
    }

    #[test]
    fn settings_store_round_trip_uses_defaults_when_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("mcp_settings.json");
        let store = McpSettingsStore::new(Some(path.clone()));

        let default_settings = store.load().expect("load defaults");
        assert!(!default_settings.enabled);
        assert_eq!(default_settings.transport, McpTransport::Stdio);

        let updated = McpSettings {
            enabled: true,
            transport: McpTransport::Http,
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            enabled_tools: vec!["list_dir".to_string(), "write_file".to_string()],
            auth_token: Some("test-token".to_string()),
            security_baseline_version: SECURITY_BASELINE_VERSION,
        };
        store.save_atomic(&updated).expect("save settings");

        let reloaded = store.load().expect("reload settings");
        assert!(reloaded.enabled);
        assert_eq!(reloaded.transport, McpTransport::Http);
        assert_eq!(reloaded.bind_address, "127.0.0.1");
        assert_eq!(reloaded.port, 0);
        assert_eq!(
            reloaded.enabled_tools,
            vec!["list_dir".to_string(), "write_file".to_string()]
        );
        assert_eq!(reloaded.auth_token.as_deref(), Some("test-token"));
        assert_eq!(
            reloaded.security_baseline_version,
            SECURITY_BASELINE_VERSION
        );
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
    }

    #[test]
    fn settings_store_normalizes_empty_and_whitespace_auth_tokens() {
        let settings = normalize_settings(McpSettings {
            auth_token: Some("   ".to_string()),
            ..McpSettings::default()
        });
        assert!(settings.auth_token.is_none());

        let settings = normalize_settings(McpSettings {
            auth_token: Some("  secret-token  ".to_string()),
            ..McpSettings::default()
        });
        assert_eq!(settings.auth_token.as_deref(), Some("secret-token"));
    }

    #[test]
    fn new_settings_has_security_baseline_version_2() {
        let settings = McpSettings::default();
        assert_eq!(
            settings.security_baseline_version,
            SECURITY_BASELINE_VERSION
        );
    }

    #[test]
    fn default_settings_contain_only_safe_tools() {
        let settings = McpSettings::default();
        assert_eq!(settings.enabled_tools, safe_tools());
    }

    #[test]
    fn legacy_settings_with_every_tool_migrates_to_safe_set() {
        let all_tools: Vec<String> = crate::server::tool_definitions()
            .iter()
            .map(|t| t.name.to_string())
            .chain(crate::server::admin_tool_names())
            .collect();

        let legacy = McpSettings {
            enabled_tools: all_tools,
            security_baseline_version: 0,
            ..McpSettings::default()
        };

        let migrated = normalize_settings(legacy);
        assert_eq!(
            migrated.security_baseline_version,
            SECURITY_BASELINE_VERSION
        );

        // Should only contain the safe default tools
        assert_eq!(migrated.enabled_tools, safe_tools());
    }

    #[test]
    fn legacy_settings_with_admin_tools_only_gets_safe_defaults() {
        let admin_only: Vec<String> = crate::server::admin_tool_names();
        let legacy = McpSettings {
            enabled_tools: admin_only,
            security_baseline_version: 0,
            ..McpSettings::default()
        };

        let migrated = normalize_settings(legacy);
        // Intersection of admin tools with safe set is empty -> use full safe set
        assert_eq!(migrated.enabled_tools, safe_tools());
    }

    #[test]
    fn legacy_settings_with_partial_safe_set_preserves_intersection() {
        let legacy_tools = vec![
            "list_dir".to_string(),
            "write_file".to_string(),
            "delete_path".to_string(),
        ];
        let legacy = McpSettings {
            enabled_tools: legacy_tools,
            security_baseline_version: 0,
            ..McpSettings::default()
        };

        let migrated = normalize_settings(legacy);
        // Only list_dir is in the intersection
        assert_eq!(migrated.enabled_tools, vec!["list_dir".to_string()]);
    }

    #[test]
    fn legacy_settings_with_no_tools_gets_safe_defaults() {
        let legacy = McpSettings {
            enabled_tools: vec![],
            security_baseline_version: 0,
            ..McpSettings::default()
        };
        let migrated = normalize_settings(legacy);
        assert_eq!(migrated.enabled_tools, safe_tools());
    }

    #[test]
    fn admin_tool_names_are_rejected_during_settings_normalization() {
        let with_admin = McpSettings {
            enabled_tools: vec![
                "list_storages".to_string(),
                "add_storage".to_string(),
                "list_dir".to_string(),
            ],
            security_baseline_version: 0,
            ..McpSettings::default()
        };
        let normalized = normalize_settings(with_admin);
        assert!(!normalized
            .enabled_tools
            .contains(&"list_storages".to_string()));
        assert!(!normalized
            .enabled_tools
            .contains(&"add_storage".to_string()));
        assert!(normalized.enabled_tools.contains(&"list_dir".to_string()));
    }

    #[test]
    fn legacy_settings_migration_creates_backup_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("mcp_settings.json");

        // Write a legacy settings file
        let legacy = McpSettings {
            enabled_tools: vec![
                "list_storages".to_string(),
                "add_storage".to_string(),
                "read_file".to_string(),
            ],
            security_baseline_version: 0,
            ..McpSettings::default()
        };
        let payload = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &payload).unwrap();

        // Load it - triggers migration
        let store = McpSettingsStore::new(Some(path.clone()));
        let migrated = store.load().expect("load with migration");

        assert_eq!(
            migrated.security_baseline_version,
            SECURITY_BASELINE_VERSION
        );
        assert!(migrated.enabled_tools.contains(&"read_file".to_string()));
        assert!(!migrated
            .enabled_tools
            .contains(&"list_storages".to_string()));

        let persisted: McpSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            persisted.security_baseline_version,
            SECURITY_BASELINE_VERSION
        );
        let backups = fs::read_dir(temp.path().join("backups"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), payload);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        assert!(backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(PRE_MIGRATION_PREFIX));
        assert_eq!(
            backups[0].extension().and_then(|value| value.to_str()),
            Some("json")
        );
    }

    #[test]
    fn migration_backup_failure_preserves_original_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("custom-settings.json");
        let store = McpSettingsStore::new(Some(path.clone()));
        let legacy = McpSettings {
            enabled_tools: vec!["list_dir".to_string(), "write_file".to_string()],
            security_baseline_version: 0,
            ..McpSettings::default()
        };
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &original).unwrap();
        fs::write(temp.path().join("backups"), b"not a directory").unwrap();

        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn already_migrated_settings_are_not_modified() {
        let tools = vec!["list_dir".to_string(), "write_file".to_string()];
        let migrated_already = McpSettings {
            enabled_tools: tools.clone(),
            security_baseline_version: SECURITY_BASELINE_VERSION,
            ..McpSettings::default()
        };
        let result = normalize_settings(migrated_already);
        assert_eq!(result.enabled_tools, tools);
        assert_eq!(result.security_baseline_version, SECURITY_BASELINE_VERSION);
    }

    #[test]
    fn already_migrated_settings_with_unknown_tool_strips_it() {
        let tools = vec!["list_dir".to_string(), "unknown_tool".to_string()];
        let migrated_already = McpSettings {
            enabled_tools: tools.clone(),
            security_baseline_version: SECURITY_BASELINE_VERSION,
            ..McpSettings::default()
        };
        let result = normalize_settings(migrated_already);
        assert_eq!(result.enabled_tools, vec!["list_dir".to_string()]);
    }
}
