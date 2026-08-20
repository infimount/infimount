use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use infimount_mcp::errors::{err_with_details, map_io_error, McpErrorCode, McpResult};
use infimount_mcp::registry::default_config_dir;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryConsent {
    #[default]
    Unknown,
    Granted,
    Denied,
}

impl<'de> Deserialize<'de> for TelemetryConsent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Legacy(bool),
            Named(String),
        }

        match Wire::deserialize(deserializer)? {
            Wire::Legacy(true) => Ok(Self::Granted),
            Wire::Legacy(false) => Ok(Self::Denied),
            Wire::Named(value) => match value.as_str() {
                "unknown" => Ok(Self::Unknown),
                "granted" => Ok(Self::Granted),
                "denied" => Ok(Self::Denied),
                _ => Err(serde::de::Error::custom("invalid telemetry consent")),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub onboarding_completed: bool,
    pub onboarding_skipped: bool,
    pub onboarding_completed_at: Option<String>,
    pub onboarding_skipped_at: Option<String>,
    pub wizard_step: Option<String>,
    pub wizard_completed_steps: Vec<String>,
    #[serde(alias = "telemetryEnabled")]
    pub telemetry_consent: TelemetryConsent,
    #[serde(default = "default_true")]
    pub local_event_persistence: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            onboarding_completed: false,
            onboarding_skipped: false,
            onboarding_completed_at: None,
            onboarding_skipped_at: None,
            wizard_step: None,
            wizard_completed_steps: Vec::new(),
            telemetry_consent: TelemetryConsent::default(),
            local_event_persistence: true,
        }
    }
}

fn default_true() -> bool {
    true
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
        settings.wizard_step = None;
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    pub fn mark_onboarding_skipped(&self) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.onboarding_skipped = true;
        settings.onboarding_skipped_at = Some(Utc::now().to_rfc3339());
        settings.wizard_step = None;
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    pub fn save_wizard_state(
        &self,
        step: Option<&str>,
        completed_steps: &[String],
    ) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.wizard_step = step.map(|s| s.to_string());
        settings.wizard_completed_steps = completed_steps.to_vec();
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    pub fn set_telemetry_consent(&self, consent: bool) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.telemetry_consent = if consent {
            TelemetryConsent::Granted
        } else {
            TelemetryConsent::Denied
        };
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    pub fn set_local_event_persistence(&self, enabled: bool) -> McpResult<AppSettings> {
        let mut settings = self.load()?;
        settings.local_event_persistence = enabled;
        self.save_atomic(&settings)?;
        Ok(settings)
    }

    #[allow(dead_code)]
    pub fn reset_all(&self) -> McpResult<AppSettings> {
        let settings = AppSettings::default();
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

        let reset = store.reset_all().unwrap();
        assert!(!reset.onboarding_completed);
        assert!(!reset.onboarding_skipped);
    }

    #[test]
    fn wizard_state_persists() {
        let dir = TempDir::new().unwrap();
        let store = AppSettingsStore::new(Some(dir.path().join("app_settings.json")));

        let steps = vec!["welcome".to_string(), "storage".to_string()];
        store
            .save_wizard_state(Some("mcp"), &steps)
            .expect("save wizard state");

        let loaded = store.load().unwrap();
        assert_eq!(loaded.wizard_step.as_deref(), Some("mcp"));
        assert_eq!(loaded.wizard_completed_steps.len(), 2);
    }

    #[test]
    fn telemetry_consent_persists() {
        let dir = TempDir::new().unwrap();
        let store = AppSettingsStore::new(Some(dir.path().join("app_settings.json")));

        store.set_telemetry_consent(true).expect("set consent");
        let loaded = store.load().unwrap();
        assert_eq!(loaded.telemetry_consent, TelemetryConsent::Granted);
    }

    #[test]
    fn telemetry_consent_migrates_legacy_boolean() {
        let parsed: AppSettings = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v0.7/app-settings.json"
        ))
        .unwrap();
        assert_eq!(parsed.telemetry_consent, TelemetryConsent::Denied);
        assert!(
            parsed.local_event_persistence,
            "legacy settings keep local persistence on"
        );
    }

    #[test]
    fn local_event_persistence_setting_round_trips() {
        let dir = TempDir::new().unwrap();
        let store = AppSettingsStore::new(Some(dir.path().join("app_settings.json")));
        assert!(store.load().unwrap().local_event_persistence);
        let disabled = store.set_local_event_persistence(false).unwrap();
        assert!(!disabled.local_event_persistence);
        assert!(!store.load().unwrap().local_event_persistence);
        let enabled = store.set_local_event_persistence(true).unwrap();
        assert!(enabled.local_event_persistence);
    }
}
