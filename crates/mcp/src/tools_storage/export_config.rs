use serde::Serialize;
use serde_json::Value;

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::tools_fs::FsToolsContext;

#[derive(Debug, Serialize)]
pub struct ExportConfigOutput {
    pub json: String,
}

#[derive(Debug, Serialize)]
struct ShareableExport {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    kind: String,
    #[serde(rename = "exportedAt")]
    exported_at: String,
    storages: Vec<ShareableStorage>,
}

#[derive(Debug, Serialize)]
struct ShareableStorage {
    name: String,
    backend: String,
    config: Value,
    #[serde(rename = "requiredSecretFields")]
    required_secret_fields: Vec<String>,
    enabled: bool,
    #[serde(rename = "mcpExposed")]
    mcp_exposed: bool,
    #[serde(rename = "readOnly")]
    read_only: bool,
    #[serde(rename = "mcpPolicy")]
    mcp_policy: Value,
}

pub async fn export_config(ctx: &FsToolsContext) -> McpResult<ExportConfigOutput> {
    let storages = ctx.registry.load_all()?;

    let shareable: Vec<ShareableStorage> = storages
        .iter()
        .map(|s| {
            let config = s.config.clone();
            let required_secret_fields: Vec<String> =
                s.secret_fields.iter().map(|f| format!("/{f}")).collect();
            ShareableStorage {
                name: s.name.clone(),
                backend: s.backend.clone(),
                config,
                required_secret_fields,
                enabled: s.enabled,
                mcp_exposed: false,
                read_only: s.read_only,
                mcp_policy: serde_json::to_value(&s.mcp_policy).unwrap_or_default(),
            }
        })
        .collect();

    let export = ShareableExport {
        schema_version: 2,
        kind: "infimount-shareable-config".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        storages: shareable,
    };

    let json = serde_json::to_string_pretty(&export).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize shareable export",
            serde_json::json!({ "serde_error": e.to_string() }),
        )
    })?;

    Ok(ExportConfigOutput { json })
}
