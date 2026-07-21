use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::{map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{enforce_storage_policy, EntryType, FsToolsContext};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatPathInput {
    pub path: String,
    #[serde(default)]
    pub session_id: Option<String>,
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
    pub user_metadata: Option<HashMap<String, String>>,
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
            user_metadata: None,
        });
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    ctx.validate_session(
        input.session_id.as_deref(),
        &resolved.storage.name,
        Some(&resolved.parsed.backend_path),
    )
    .await?;
    enforce_storage_policy(
        &resolved.storage,
        &resolved.parsed.backend_path,
        McpOperation::Metadata,
        false,
        false,
    )?;
    let op = opendal_adapter::build_operator(&resolved.storage, &ctx.registry)?;

    if parsed.backend_path.is_empty() {
        return Ok(StatPathOutput {
            path: parsed.normalized,
            entry_type: EntryType::Dir,
            size_bytes: None,
            modified_at: None,
            etag: None,
            content_type: None,
            user_metadata: None,
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
        user_metadata: meta.user_metadata().cloned(),
    })
}
