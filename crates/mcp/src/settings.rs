use crate::errors::{err, err_with_details, map_io_error, McpErrorCode, McpResult};
use crate::registry::default_config_dir;
use crate::server::default_enabled_tool_names;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::SystemTime;
use std::time::{Duration, Instant};

const SETTINGS_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_HTTP_BIND_ADDRESS: &str = "127.0.0.1";
pub const DEFAULT_HTTP_PORT: u16 = 7331;

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
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: McpTransport::Stdio,
            bind_address: DEFAULT_HTTP_BIND_ADDRESS.to_string(),
            port: DEFAULT_HTTP_PORT,
            enabled_tools: default_enabled_tool_names(),
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
        Ok(normalize_settings(settings))
    }

    fn save_atomic_unlocked(&self, settings: &McpSettings) -> McpResult<()> {
        ensure_parent(&self.path)?;

        let parent = self.path.parent().ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "settings path has no parent directory",
                serde_json::json!({ "path": self.path }),
            )
        })?;
        let tmp_path = parent.join(format!(
            ".mcp_settings.json.tmp.{}.{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));

        let normalized = normalize_settings(settings.clone());
        let payload = serde_json::to_vec_pretty(&normalized).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize MCP settings",
                serde_json::json!({ "serde_error": e.to_string() }),
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
                        "timed out acquiring MCP settings lock",
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

pub fn default_settings_path() -> PathBuf {
    default_config_dir().join("mcp_settings.json")
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
    settings.enabled_tools = sanitize_enabled_tools(settings.enabled_tools);
    settings
}

fn sanitize_enabled_tools(enabled_tools: Vec<String>) -> Vec<String> {
    let allowed = default_enabled_tool_names()
        .into_iter()
        .collect::<HashSet<_>>();
    let mut normalized = enabled_tools
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && allowed.contains(value))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

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
            enabled_tools: vec!["list_dir".to_string(), "export_config".to_string()],
        };
        store.save_atomic(&updated).expect("save settings");

        let reloaded = store.load().expect("reload settings");
        assert!(reloaded.enabled);
        assert_eq!(reloaded.transport, McpTransport::Http);
        assert_eq!(reloaded.bind_address, "127.0.0.1");
        assert_eq!(reloaded.port, 0);
        assert_eq!(
            reloaded.enabled_tools,
            vec!["export_config".to_string(), "list_dir".to_string()]
        );
        assert!(path.exists());
    }
}
