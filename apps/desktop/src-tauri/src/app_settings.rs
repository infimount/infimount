use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use infimount_mcp::errors::{err_with_details, map_io_error, McpErrorCode, McpResult};
use infimount_mcp::registry::default_config_dir;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub onboarding_completed: bool,
    pub onboarding_skipped: bool,
    pub onboarding_completed_at: Option<String>,
    pub onboarding_skipped_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppSettingsStore {
    path: PathBuf,
}

impl AppSettingsStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.unwrap_or_else(default_app_settings_path),
        }
    }

    pub fn load(&self) -> McpResult<AppSettings> {
        if !self.path.exists() {
            return Ok(AppSettings::default());
        }
        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        serde_json::from_str(&data).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse app settings",
                json!({ "serde_error": e.to_string(), "path": self.path }),
            )
        })
    }

    pub fn save_atomic(&self, settings: &AppSettings) -> McpResult<()> {
        let payload = serde_json::to_vec_pretty(settings).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize app settings",
                json!({ "serde_error": e.to_string() }),
            )
        })?;
        infimount_core::atomic_file::atomic_write_file(
            &self.path,
            &payload,
            infimount_core::atomic_file::FILE_MODE,
        )
        .map_err(|error| infimount_mcp::errors::map_core_error(&error))
    }

    pub fn mark_onboarding_completed(&self) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.onboarding_completed = true;
        settings.onboarding_skipped = false;
        settings.onboarding_completed_at = Some(Utc::now().to_rfc3339());
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    pub fn mark_onboarding_skipped(&self) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.onboarding_skipped = true;
        settings.onboarding_skipped_at = Some(Utc::now().to_rfc3339());
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    #[cfg(test)]
    pub fn reset_onboarding(&self) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.onboarding_completed = false;
        settings.onboarding_skipped = false;
        settings.onboarding_completed_at = None;
        settings.onboarding_skipped_at = None;
        self.save_atomic(&settings)?;
        Ok(settings)
    }
}

pub fn default_app_settings_path() -> PathBuf {
    default_config_dir().join("app_settings.json")
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn app_settings_store_round_trips_onboarding_state() {
        let dir = TempDir::new().unwrap();
        let store = AppSettingsStore::new(Some(dir.path().join("app_settings.json")));

        assert!(!store.load().unwrap().onboarding_completed);
        let completed = store.mark_onboarding_completed().unwrap();
        assert!(completed.onboarding_completed);
        assert!(!completed.onboarding_skipped);

        let skipped = store.mark_onboarding_skipped().unwrap();
        assert!(skipped.onboarding_skipped);
        assert!(skipped.onboarding_completed);

        let reset = store.reset_onboarding().unwrap();
        assert!(!reset.onboarding_completed);
        assert!(!reset.onboarding_skipped);
    }
}
