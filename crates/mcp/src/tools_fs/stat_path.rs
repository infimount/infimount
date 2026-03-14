use serde::{Deserialize, Serialize};

use crate::errors::{map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{EntryType, FsToolsContext};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatPathInput {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct StatPathOutput {
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
}

pub async fn stat_path(ctx: &FsToolsContext, input: StatPathInput) -> McpResult<StatPathOutput> {
    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::StatPath, &parsed)?;

    if parsed.is_root {
        return Ok(StatPathOutput {
            path: "/".to_string(),
            entry_type: EntryType::Dir,
            size_bytes: None,
            modified_at: None,
            etag: None,
            content_type: None,
        });
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let op = opendal_adapter::build_operator(&resolved.storage)?;

    if parsed.backend_path.is_empty() {
        return Ok(StatPathOutput {
            path: parsed.normalized,
            entry_type: EntryType::Dir,
            size_bytes: None,
            modified_at: None,
            etag: None,
            content_type: None,
        });
    }

    let meta = op
        .stat(&parsed.backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;

    Ok(StatPathOutput {
        path: parsed.normalized,
        entry_type: if meta.is_dir() {
            EntryType::Dir
        } else {
            EntryType::File
        },
        size_bytes: if meta.is_dir() {
            None
        } else {
            Some(meta.content_length())
        },
        modified_at: meta.last_modified().map(|dt| dt.to_string()),
        etag: meta.etag().map(|s| s.to_string()),
        content_type: meta.content_type().map(|s| s.to_string()),
    })
}
