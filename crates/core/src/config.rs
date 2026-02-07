use std::fs;
use std::path::PathBuf;

use crate::models::{Result, Source};

/// Location of the configuration file.
///
/// Uses `~/.infimount/config.json` by default, or a custom
/// path via the `INFIMOUNT_CONFIG` env var.
fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("INFIMOUNT_CONFIG") {
        return PathBuf::from(p);
    }

    default_config_path().unwrap_or_else(|| PathBuf::from(".infimount").join("config.json"))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn home_dir() -> Option<String> {
    non_empty_env("HOME").or_else(|| non_empty_env("USERPROFILE"))
}

fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|home| PathBuf::from(home).join(".infimount").join("config.json"))
}

/// Load all configured sources.
pub fn load_sources() -> Result<Vec<Source>> {
    let path = config_path();
    if !path.exists() {
        // No config yet; start with an empty list of sources.
        return Ok(Vec::new());
    }

    let data = fs::read_to_string(&path)?;
    let sources: Vec<Source> = serde_json::from_str(&data)?;
    Ok(sources)
}

/// Persist the current list of sources.
pub fn save_sources(sources: &[Source]) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let data = serde_json::to_string_pretty(sources)?;
    fs::write(path, data)?;
    Ok(())
}
