use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::FsToolsContext;

fn default_expires_seconds() -> u64 {
    900
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateDownloadLinkInput {
    pub path: String,
    #[serde(default = "default_expires_seconds")]
    pub expires_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct GenerateDownloadLinkOutput {
    pub path: String,
    pub url: String,
    pub expires_at: String,
}

pub async fn generate_download_link(
    ctx: &FsToolsContext,
    input: GenerateDownloadLinkInput,
) -> McpResult<GenerateDownloadLinkOutput> {
    if input.expires_seconds < 60 || input.expires_seconds > 86_400 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "expires_seconds must be between 60 and 86400",
            json!({ "expires_seconds": input.expires_seconds }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::GenerateDownloadLink, &parsed)?;
    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let op = opendal_adapter::build_operator(&resolved.storage)?;

    if parsed.backend_path.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let meta = op
        .stat(&parsed.backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    if meta.is_dir() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let caps = op.info().full_capability();
    if !caps.presign_read {
        return Err(err_with_details(
            McpErrorCode::ERR_PRESIGN_NOT_SUPPORTED,
            "backend does not support presigned read URLs",
            json!({ "path": parsed.normalized, "backend": resolved.storage.backend }),
        ));
    }

    let presigned = op
        .presign_read(
            &parsed.backend_path,
            Duration::from_secs(input.expires_seconds),
        )
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;

    Ok(GenerateDownloadLinkOutput {
        path: parsed.normalized,
        url: presigned.uri().to_string(),
        expires_at: (Utc::now() + ChronoDuration::seconds(input.expires_seconds as i64))
            .to_rfc3339(),
    })
}
