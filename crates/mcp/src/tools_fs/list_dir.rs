use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{
    collect_entries, default_limit, sort_entries, EntryType, FsToolsContext, ListDirEntry,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDirInput {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListDirOutput {
    pub path: String,
    pub entries: Vec<ListDirEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorV1 {
    v: u8,
    offset: usize,
}

pub async fn list_dir(ctx: &FsToolsContext, input: ListDirInput) -> McpResult<ListDirOutput> {
    if input.limit == 0 || input.limit > 1000 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "limit must be between 1 and 1000",
            json!({ "limit": input.limit }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ListDir, &parsed)?;
    let offset = decode_cursor(input.cursor.as_deref())?;

    if parsed.is_root {
        let entries = ctx
            .registry
            .list_exposed_enabled()?
            .into_iter()
            .map(|storage| ListDirEntry {
                name: storage.name.clone(),
                path: format!("/{}", storage.name),
                entry_type: EntryType::Dir,
                size_bytes: None,
                modified_at: None,
                etag: None,
            })
            .collect::<Vec<_>>();

        return Ok(paginate_entries(
            parsed.normalized,
            entries,
            offset,
            input.limit as usize,
        ));
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let op = opendal_adapter::build_operator(&resolved.storage)?;

    if !parsed.backend_path.is_empty() {
        let meta = op
            .stat(&parsed.backend_path)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
        if !meta.is_dir() {
            return Err(err_with_details(
                McpErrorCode::ERR_NOT_A_DIRECTORY,
                "path is not a directory",
                json!({ "path": parsed.normalized }),
            ));
        }
    }

    let mut entries = collect_entries(
        &op,
        &resolved.storage.name,
        &parsed.backend_path,
        input.recursive,
    )
    .await?;
    sort_entries(&mut entries, input.recursive);

    Ok(paginate_entries(
        parsed.normalized,
        entries,
        offset,
        input.limit as usize,
    ))
}

fn paginate_entries(
    path: String,
    entries: Vec<ListDirEntry>,
    offset: usize,
    limit: usize,
) -> ListDirOutput {
    let start = offset.min(entries.len());
    let end = (start + limit).min(entries.len());
    let page = entries[start..end].to_vec();
    let next_cursor = if end < entries.len() {
        Some(encode_cursor(end))
    } else {
        None
    };

    ListDirOutput {
        path,
        entries: page,
        next_cursor,
    }
}

fn decode_cursor(cursor: Option<&str>) -> McpResult<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };

    // Cursor contract for v1:
    // base64url(JSON) with shape {"v":1,"offset":n}.
    // Offset is interpreted against the same (path, recursive) query shape,
    // after deterministic sorting and filtering have been applied.
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

fn encode_cursor(offset: usize) -> String {
    let payload = serde_json::to_vec(&CursorV1 { v: 1, offset }).unwrap_or_else(|_| b"{}".to_vec());
    URL_SAFE_NO_PAD.encode(payload)
}
