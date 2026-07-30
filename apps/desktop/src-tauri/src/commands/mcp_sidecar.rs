use serde::Serialize;

use crate::activation_probe::verified_sidecar_path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSidecarInfo {
    pub bundled_path: Option<String>,
    pub available: bool,
    pub error_code: Option<String>,
}

#[tauri::command]
pub fn get_mcp_sidecar_info() -> McpSidecarInfo {
    match verified_sidecar_path() {
        Ok(path) => McpSidecarInfo {
            bundled_path: Some(path.to_string_lossy().to_string()),
            available: true,
            error_code: None,
        },
        Err(code) => McpSidecarInfo {
            bundled_path: None,
            available: false,
            error_code: Some(code.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_info_never_returns_an_unverified_path() {
        let info = get_mcp_sidecar_info();
        assert_eq!(info.available, info.bundled_path.is_some());
        assert_eq!(info.available, info.error_code.is_none());
    }
}
