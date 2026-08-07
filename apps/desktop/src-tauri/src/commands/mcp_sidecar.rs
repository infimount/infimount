use serde::Serialize;

use crate::activation_probe::validate_sidecar_binary;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSidecarInfo {
    pub bundled_path: Option<String>,
    pub available: bool,
    pub executable: bool,
    pub desktop_version: String,
    pub sidecar_version: Option<String>,
    pub compatible: bool,
    pub sha256: Option<String>,
    pub checksum_verified: bool,
    pub doctor_healthy: bool,
    pub error_code: Option<String>,
}

#[tauri::command]
pub fn get_mcp_sidecar_info() -> McpSidecarInfo {
    let validation = validate_sidecar_binary();
    let available = validation.binary_found
        && validation.executable
        && validation.version_match
        && validation.doctor_healthy;
    McpSidecarInfo {
        bundled_path: validation.canonical_path,
        available,
        executable: validation.executable,
        desktop_version: env!("CARGO_PKG_VERSION").to_string(),
        sidecar_version: validation.version,
        compatible: validation.version_match,
        sha256: validation.sha256,
        checksum_verified: validation.checksum_verified,
        doctor_healthy: validation.doctor_healthy,
        error_code: validation.error_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_info_never_returns_an_unverified_path() {
        let info = get_mcp_sidecar_info();
        assert_eq!(info.desktop_version, env!("CARGO_PKG_VERSION"));
        if info.available {
            assert!(info.bundled_path.is_some());
            assert!(info.executable);
            assert!(info.compatible);
            assert!(info.doctor_healthy);
            assert!(info.sha256.is_some());
            assert!(info.error_code.is_none());
        }
    }
}
