use std::fs;
use std::path::PathBuf;

use crate::models::{Result, Source};

/// Location of the configuration file.
///
/// For now this is a simple `openhsb.json` in the current working
/// directory, or a custom path via the `OPENHSB_CONFIG` env var.
fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("OPENHSB_CONFIG") {
        return PathBuf::from(p);
    }
    PathBuf::from("openhsb.json")
}

/// Load all configured sources from `openhsb.json`.
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

/// Persist the current list of sources to `openhsb.json`.
pub fn save_sources(sources: &[Source]) -> Result<()> {
    let path = config_path();
    let data = serde_json::to_string_pretty(sources)?;
    fs::write(path, data)?;
    Ok(())
}

