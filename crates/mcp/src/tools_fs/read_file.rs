use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{default_as_text, default_encoding, default_read_max_bytes, FsToolsContext};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileInput {
    pub path: String,
    #[serde(default)]
    pub offset_bytes: u64,
    #[serde(default = "default_read_max_bytes")]
    pub max_bytes: u32,
    #[serde(default = "default_as_text")]
    pub as_text: bool,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Serialize)]
pub struct ReadFileOutput {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub read_bytes: u64,
}

pub async fn read_file(ctx: &FsToolsContext, input: ReadFileInput) -> McpResult<ReadFileOutput> {
    if input.max_bytes == 0 || input.max_bytes > 2_097_152 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "max_bytes must be between 1 and 2097152",
            json!({ "max_bytes": input.max_bytes }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ReadFile, &parsed)?;
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

    let size = meta.content_length();
    if input.offset_bytes >= size {
        return Ok(ReadFileOutput {
            path: parsed.normalized,
            content: String::new(),
            truncated: false,
            read_bytes: 0,
        });
    }

    let requested_end = input.offset_bytes.saturating_add(input.max_bytes as u64);
    let truncated = size > requested_end;
    let read_end = requested_end.min(size);

    let bytes = op
        .read_with(&parsed.backend_path)
        .range(input.offset_bytes..read_end)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?
        .to_vec();

    let read_bytes = bytes.len() as u64;
    let content = if input.as_text {
        if !input.encoding.eq_ignore_ascii_case("utf-8") {
            return Err(err_with_details(
                McpErrorCode::ERR_TEXT_DECODE_FAILED,
                "failed to decode file as text",
                json!({
                    "path": parsed.normalized,
                    "encoding": input.encoding,
                    "supported_encoding": "utf-8",
                    "hint": "use as_text=false"
                }),
            ));
        }
        String::from_utf8(bytes).map_err(|_| {
            err_with_details(
                McpErrorCode::ERR_TEXT_DECODE_FAILED,
                "failed to decode file as text",
                json!({
                    "path": parsed.normalized,
                    "supported_encoding": "utf-8",
                    "hint": "use as_text=false"
                }),
            )
        })?
    } else {
        BASE64_STANDARD.encode(bytes)
    };

    Ok(ReadFileOutput {
        path: parsed.normalized,
        content,
        truncated,
        read_bytes,
    })
}
