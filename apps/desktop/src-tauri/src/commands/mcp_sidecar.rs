use serde::Serialize;

fn sidecar_path() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Look for any mcp-* binary next to the executable
    for entry in std::fs::read_dir(&exe_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("mcp-") {
            return Some(entry.path());
        }
    }
    // Fall back: check binaries/ subdirectory (dev mode)
    let binaries_dir = exe_dir.join("binaries");
    if let Ok(entries) = std::fs::read_dir(&binaries_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("mcp-") {
                return Some(entry.path());
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSidecarInfo {
    pub bundled_path: Option<String>,
    pub available: bool,
}

#[tauri::command]
pub fn get_mcp_sidecar_info() -> McpSidecarInfo {
    match sidecar_path() {
        Some(path) => {
            let available = path.exists();
            McpSidecarInfo {
                bundled_path: Some(path.to_string_lossy().to_string()),
                available,
            }
        }
        None => McpSidecarInfo {
            bundled_path: None,
            available: false,
        },
    }
}
