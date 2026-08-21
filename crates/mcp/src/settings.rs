use crate::errors::{
    err, err_with_details, map_io_error, sanitized_parse_error, McpErrorCode, McpResult,
};
use crate::registry::default_config_dir;
use crate::server::{all_tool_names, default_enabled_tool_names};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;
use std::time::{Duration, Instant};

const SETTINGS_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_HTTP_BIND_ADDRESS: &str = "127.0.0.1";
pub const DEFAULT_HTTP_PORT: u16 = 7331;

pub const SECURITY_BASELINE_VERSION: u32 = 2;
pub const MCP_SETTINGS_SCHEMA_VERSION: u32 = 2;
pub const MCP_AUTH_TOKEN_ACCOUNT: &str = "mcp/http-auth";

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
    #[serde(default)]
    pub schema_version: u32,
    pub enabled: bool,
    pub transport: McpTransport,
    pub bind_address: String,
    pub port: u16,
    #[serde(default = "default_enabled_tool_names")]
    pub enabled_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token_ref: Option<String>,
    #[serde(default, skip_serializing)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub security_baseline_version: u32,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            schema_version: MCP_SETTINGS_SCHEMA_VERSION,
            enabled: false,
            transport: McpTransport::Stdio,
            bind_address: DEFAULT_HTTP_BIND_ADDRESS.to_string(),
            port: DEFAULT_HTTP_PORT,
            enabled_tools: default_enabled_tool_names(),
            auth_token_ref: None,
            auth_token: None,
            security_baseline_version: SECURITY_BASELINE_VERSION,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpSettingsStore {
    path: PathBuf,
    lock_path: PathBuf,
    secret_store: Arc<dyn infimount_core::secrets::SecretStore>,
}

impl McpSettingsStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::with_secret_store(
            path,
            Arc::new(infimount_core::secrets::NativeSecretStore::new()),
        )
    }

    pub fn with_secret_store(
        path: Option<PathBuf>,
        secret_store: Arc<dyn infimount_core::secrets::SecretStore>,
    ) -> Self {
        let path = path.unwrap_or_else(default_settings_path);
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
            sanitized_parse_error(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse MCP settings",
                "invalid_mcp_settings",
                &e,
            )
        })?;
        let legacy_token = settings
            .auth_token
            .clone()
            .filter(|token| !token.trim().is_empty());
        let requires_migration = settings.schema_version < MCP_SETTINGS_SCHEMA_VERSION
            || settings.security_baseline_version < SECURITY_BASELINE_VERSION
            || legacy_token.is_some();
        let mut normalized = normalize_settings(settings);
        normalized.schema_version = MCP_SETTINGS_SCHEMA_VERSION;
        if requires_migration {
            let _backup_path = self.create_pre_migration_backup(data.as_bytes())?;
            let previous = if legacy_token.is_some() {
                self.secret_store
                    .get_json(MCP_AUTH_TOKEN_ACCOUNT)
                    .map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "failed to stage HTTP auth migration",
                        )
                    })?
            } else {
                None
            };
            let changed_secret = if let Some(token) = legacy_token {
                if self
                    .secret_store
                    .put_json(MCP_AUTH_TOKEN_ACCOUNT, &serde_json::json!({"token": token}))
                    .is_err()
                {
                    restore_secret(
                        self.secret_store.as_ref(),
                        MCP_AUTH_TOKEN_ACCOUNT,
                        previous.as_ref(),
                    )?;
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to migrate HTTP auth token",
                    ));
                }
                normalized.auth_token_ref = Some(MCP_AUTH_TOKEN_ACCOUNT.to_string());
                true
            } else {
                false
            };
            if let Err(error) = self.save_atomic_unlocked(&normalized) {
                if changed_secret {
                    restore_secret(
                        self.secret_store.as_ref(),
                        MCP_AUTH_TOKEN_ACCOUNT,
                        previous.as_ref(),
                    )?;
                }
                return Err(error);
            }
            let verification = fs::read(&self.path)
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "HTTP auth migration verification failed",
                    )
                })
                .and_then(|bytes| {
                    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "HTTP auth migration verification failed",
                        )
                    })
                });
            let verified = verification.as_ref().is_ok_and(|persisted| {
                persisted.get("authToken").is_none()
                    && persisted
                        .get("schemaVersion")
                        .and_then(|value| value.as_u64())
                        == Some(MCP_SETTINGS_SCHEMA_VERSION as u64)
            });
            if !verified {
                write_sensitive_atomic(&self.path, data.as_bytes())?;
                if changed_secret {
                    restore_secret(
                        self.secret_store.as_ref(),
                        MCP_AUTH_TOKEN_ACCOUNT,
                        previous.as_ref(),
                    )?;
                }
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "HTTP auth migration verification failed",
                ));
            }
            crate::migration_cleanup::delete_plaintext_backup_or_journal(&_backup_path)?;
        }
        Ok(normalized)
    }

    fn save_atomic_unlocked(&self, settings: &McpSettings) -> McpResult<()> {
        ensure_parent(&self.path)?;

        let mut to_save = settings.clone();
        to_save.auth_token = None;

        let normalized = normalize_settings(to_save);
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
        infimount_core::atomic_file::create_dir_all(&backups_dir)
            .map_err(|error| crate::errors::map_core_error(&error))?;
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
    infimount_core::atomic_file::atomic_write_file(
        path,
        payload,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|error| crate::errors::map_core_error(&error))
}

fn ensure_parent(path: &Path) -> McpResult<()> {
    infimount_core::atomic_file::ensure_parent(path)
        .map_err(|error| crate::errors::map_core_error(&error))
}

fn normalize_settings(mut settings: McpSettings) -> McpSettings {
    settings = migrate_security_baseline(settings);
    settings.enabled_tools = sanitize_enabled_tools(settings.enabled_tools);
    settings.auth_token = None;
    settings.auth_token_ref = normalize_auth_token_ref(settings.auth_token_ref);
    settings
}

pub fn normalize_auth_token_ref(value: Option<String>) -> Option<String> {
    value
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

pub fn normalize_auth_token(value: Option<String>) -> Option<String> {
    normalize_auth_token_ref(value)
}

fn restore_secret(
    secret_store: &dyn infimount_core::secrets::SecretStore,
    account: &str,
    previous: Option<&serde_json::Value>,
) -> McpResult<()> {
    let restored = match previous {
        Some(value) => secret_store.put_json(account, value),
        None => secret_store.delete(account),
    };
    restored.map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "HTTP credential rollback failed; manual secret-store cleanup is required",
        )
    })
}

pub fn resolve_auth_token(
    auth_token_ref: &Option<String>,
    secret_store: &dyn infimount_core::secrets::SecretStore,
) -> McpResult<Option<String>> {
    if let Some(account) = auth_token_ref {
        let bundle = secret_store
            .get_json(account)
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "native secret storage is unavailable",
                )
            })?
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_SECRET_NOT_FOUND,
                    "configured HTTP auth token is missing",
                )
            })?;
        let token = bundle
            .get("token")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty() && *value != "********")
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_SECRET_NOT_FOUND,
                    "configured HTTP auth token is invalid",
                )
            })?;
        return Ok(Some(token.to_string()));
    }
    Ok(std::env::var("INFIMOUNT_AUTH_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty()))
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
    use infimount_core::secrets::SecretStore;

    fn safe_tools() -> Vec<String> {
        default_enabled_tool_names()
    }

    #[test]
    fn legacy_plaintext_auth_token_migrates_transactionally() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("mcp_settings.json");
        let secret_store = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let store = McpSettingsStore::with_secret_store(Some(path.clone()), secret_store.clone());
        let original = include_str!("../../../tests/fixtures/v0.7/mcp-settings-all-tools.json");
        fs::write(&path, original).unwrap();

        let loaded = store.load().expect("migrate settings");
        assert_eq!(
            loaded.auth_token_ref.as_deref(),
            Some(MCP_AUTH_TOKEN_ACCOUNT)
        );
        assert!(loaded.auth_token.is_none());
        assert_eq!(
            secret_store
                .get_json(MCP_AUTH_TOKEN_ACCOUNT)
                .unwrap()
                .unwrap()["token"],
            "TEST_HTTP_BEARER_TOKEN_DO_NOT_SHIP"
        );
        let persisted = fs::read_to_string(path).unwrap();
        assert!(!persisted.contains("TEST_HTTP_BEARER_TOKEN_DO_NOT_SHIP"));
        assert!(!loaded
            .enabled_tools
            .iter()
            .any(|tool| tool == "add_storage"));
    }

    #[test]
    fn non_secret_settings_migration_does_not_require_keyring() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("mcp_settings.json");
        let secret_store = Arc::new(infimount_core::secrets::UnavailableSecretStore::new(
            "locked",
        ));
        let store = McpSettingsStore::with_secret_store(Some(path.clone()), secret_store);
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "enabled": false,
                "transport": "stdio",
                "bindAddress": "127.0.0.1",
                "port": 7331,
                "enabledTools": ["list_dir"],
                "securityBaselineVersion": 1
            }))
            .unwrap(),
        )
        .unwrap();

        let migrated = store.load().expect("non-secret migration");
        assert_eq!(migrated.schema_version, MCP_SETTINGS_SCHEMA_VERSION);
        assert!(migrated.auth_token_ref.is_none());
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
            schema_version: MCP_SETTINGS_SCHEMA_VERSION,
            enabled: true,
            transport: McpTransport::Http,
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            enabled_tools: vec!["list_dir".to_string(), "write_file".to_string()],
            auth_token_ref: Some("mcp/http-auth".to_string()),
            auth_token: None,
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
        assert_eq!(reloaded.auth_token_ref.as_deref(), Some("mcp/http-auth"));
        assert!(reloaded.auth_token.is_none());
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
    fn settings_store_normalizes_empty_and_whitespace_auth_token_refs() {
        let settings = normalize_settings(McpSettings {
            auth_token_ref: Some("   ".to_string()),
            ..McpSettings::default()
        });
        assert!(settings.auth_token_ref.is_none());

        let settings = normalize_settings(McpSettings {
            auth_token_ref: Some("  mcp/http-auth  ".to_string()),
            ..McpSettings::default()
        });
        assert_eq!(settings.auth_token_ref.as_deref(), Some("mcp/http-auth"));
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
        // Plaintext pre-migration backup is deleted after successful migration
        let backups_path = temp.path().join("backups");
        if backups_path.exists() {
            let count = fs::read_dir(&backups_path)
                .unwrap()
                .filter_map(|e| e.ok())
                .count();
            assert_eq!(count, 0, "backup file should be removed after success");
        }
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
