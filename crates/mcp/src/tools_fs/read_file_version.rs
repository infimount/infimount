use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{
    core_error_to_mcp_error, default_as_text, default_encoding, enforce_storage_policy,
    resolve_read_max_bytes, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileVersionInput {
    pub path: String,
    pub version: String,
    #[serde(default)]
    pub offset_bytes: u64,
    #[serde(default)]
    pub max_bytes: Option<u32>,
    #[serde(default = "default_as_text")]
    pub as_text: bool,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadFileVersionOutput {
    pub path: String,
    pub version: String,
    pub content: String,
    pub truncated: bool,
    pub read_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

pub async fn read_file_version(
    ctx: &FsToolsContext,
    input: ReadFileVersionInput,
) -> McpResult<ReadFileVersionOutput> {
    let max_bytes = resolve_read_max_bytes(ctx, input.max_bytes)?;

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ReadFileVersion, &parsed)?;

    if parsed.backend_path.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
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
    enforce_storage_policy(
        &resolved.storage,
        &resolved.parsed.backend_path,
        McpOperation::ReadFileVersion,
        false,
        false,
    )?;
    let op = opendal_adapter::build_operator(&resolved.storage, &ctx.registry)?;

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
    if !capabilities.read_with_version {
        return Err(err_with_details(
            McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED,
            "version read not supported for this storage backend",
            json!({
                "path": parsed.normalized,
                "storage": resolved.storage.name
            }),
        ));
    }

    let meta = op
        .stat_with(&parsed.backend_path)
        .version(&input.version)
        .await
        .map_err(|e: opendal::Error| {
            if e.kind() == opendal::ErrorKind::NotFound {
                return map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND);
            }
            map_opendal_error(&e, McpErrorCode::ERR_INTERNAL)
        })?;

    if meta.is_dir() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let content_type = meta
        .content_type()
        .filter(|value| infimount_core::operations::valid_content_type(value))
        .map(str::to_owned);
    let size = meta.content_length();
    if input.offset_bytes >= size {
        return Ok(ReadFileVersionOutput {
            path: parsed.normalized,
            version: input.version,
            content: String::new(),
            truncated: false,
            read_bytes: 0,
            content_type,
        });
    }

    let requested_end = input.offset_bytes.saturating_add(max_bytes as u64);
    let truncated = size > requested_end;
    let read_end = requested_end.min(size);

    let bytes = infimount_core::operations::read_range_bounded(
        &op,
        &parsed.backend_path,
        input.offset_bytes,
        read_end,
        Some(&input.version),
    )
    .await
    .map_err(core_error_to_mcp_error)?;

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

    Ok(ReadFileVersionOutput {
        path: parsed.normalized,
        version: input.version,
        content,
        truncated,
        read_bytes,
        content_type,
    })
}
