use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{default_limit, FsToolsContext};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListVersionsInput {
    pub path: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListVersionsOutput {
    pub path: String,
    pub versions: Vec<VersionEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CursorV1 {
    v: u8,
    offset: usize,
}

pub async fn list_versions(
    ctx: &FsToolsContext,
    input: ListVersionsInput,
) -> McpResult<ListVersionsOutput> {
    if input.limit == 0 || input.limit > 1000 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "limit must be between 1 and 1000",
            json!({ "limit": input.limit }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ListVersions, &parsed)?;
    let offset = decode_cursor(input.cursor.as_deref())?;

    if parsed.is_root {
        return Err(err_with_details(
            McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED,
            "cannot list versions at root",
            json!({ "path": parsed.normalized }),
        ));
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    ctx.validate_session(
        input.session_id.as_deref(),
        &resolved.storage.name,
        Some(&resolved.parsed.backend_path),
    )
    .await?;
    let op = opendal_adapter::build_operator(&resolved.storage)?;

    if let Some(disabled) = opendal_adapter::check_versioning_disabled(&resolved.storage) {
        if disabled {
            return Err(err_with_details(
                McpErrorCode::ERR_VERSIONS_NOT_ENABLED,
                "versioning is explicitly disabled in storage configuration",
                json!({
                    "path": parsed.normalized,
                    "storage": resolved.storage.name,
                    "hint": "enable versioning in storage config to use version tools"
                }),
            ));
        }
    }

    let capabilities = opendal_adapter::get_capabilities(&op);
    if !capabilities.list_with_versions {
        return Err(err_with_details(
            McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED,
            "version listing not supported for this storage backend",
            json!({
                "path": parsed.normalized,
                "storage": resolved.storage.name
            }),
        ));
    }

    let meta = op
        .stat(&parsed.backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND))?;

    if meta.is_dir() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "cannot list versions for a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let versions = collect_versions(&op, &parsed.backend_path).await?;

    let mut sorted_versions = versions;
    sorted_versions.sort_by(|a, b| {
        let a_time = a.modified_at.as_deref().unwrap_or("");
        let b_time = b.modified_at.as_deref().unwrap_or("");
        b_time.cmp(a_time).then_with(|| a.version.cmp(&b.version))
    });

    let start = offset.min(sorted_versions.len());
    let end = (start + input.limit as usize).min(sorted_versions.len());
    let page = sorted_versions[start..end].to_vec();
    let next_cursor = if end < sorted_versions.len() {
        Some(encode_cursor(end))
    } else {
        None
    };

    Ok(ListVersionsOutput {
        path: parsed.normalized,
        versions: page,
        next_cursor,
    })
}

async fn collect_versions(
    op: &opendal::Operator,
    backend_path: &str,
) -> McpResult<Vec<VersionEntry>> {
    let mut versions = Vec::new();

    let lister = op
        .lister_with(backend_path)
        .versions(true)
        .await
        .map_err(|e| {
            if e.kind() == opendal::ErrorKind::Unsupported {
                err_with_details(
                    McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED,
                    "version listing not supported for this storage backend",
                    json!({ "backend_error": e.to_string() }),
                )
            } else {
                map_opendal_error(&e, McpErrorCode::ERR_INTERNAL)
            }
        })?;

    let mut stream = lister;
    while let Some(entry_result) = stream.next().await {
        let entry = entry_result.map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;

        let meta = entry.metadata();
        let version = meta
            .version()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "default".to_string());

        if version != "default" {
            let modified_at = meta.last_modified().map(|t| t.to_string());
            let etag = meta.etag().map(|e| e.to_string());
            versions.push(VersionEntry {
                version,
                size_bytes: Some(meta.content_length()),
                modified_at,
                etag,
            });
        }
    }

    Ok(versions)
}

pub(crate) fn decode_cursor(cursor: Option<&str>) -> McpResult<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };

    let raw = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor encoding",
            json!({ "cursor": cursor }),
        )
    })?;

    let parsed: CursorV1 = serde_json::from_slice(&raw).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor payload",
            json!({ "cursor": cursor }),
        )
    })?;

    if parsed.v != 1 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "unsupported cursor version",
            json!({ "cursor_version": parsed.v }),
        ));
    }

    Ok(parsed.offset)
}

pub(crate) fn encode_cursor(offset: usize) -> String {
    let payload = serde_json::to_vec(&CursorV1 { v: 1, offset }).unwrap_or_else(|_| b"{}".to_vec());
    URL_SAFE_NO_PAD.encode(payload)
}
