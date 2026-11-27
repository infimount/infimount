use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::errors::{err_with_details, map_opendal_error, McpError, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{
    core_error_to_mcp_error, create_dir_chain, default_encoding, default_true,
    enforce_storage_policy, missing_directory_paths, parent_path, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteFileInput {
    pub path: String,
    pub content: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_true")]
    pub overwrite: bool,
    #[serde(default)]
    pub create_parents: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub confirmation_id: Option<String>,
    #[serde(default)]
    pub user_metadata: Option<HashMap<String, String>>,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteFileOutput {
    pub path: String,
    pub written_bytes: u64,
}

/// Maximum number of bytes an MCP `write_file` request may write (after decoding).
pub const MAX_MCP_WRITE_BYTES: usize = crate::settings::MAX_MCP_WRITE_BYTES as usize;

/// Longest padded base64 text whose decoded bytes can fit in
/// [`MAX_MCP_WRITE_BYTES`]. Shared by the runtime pre-check and the schema
/// `maxLength`, so the two cannot drift.
pub const MAX_MCP_WRITE_BASE64_LEN: usize = MAX_MCP_WRITE_BYTES.div_ceil(3) * 4;

pub async fn write_file(
    ctx: &FsToolsContext,
    mut input: WriteFileInput,
) -> McpResult<WriteFileOutput> {
    ctx.byte_limits.validate()?;
    validate_content_type(input.content_type.as_deref())?;
    validate_write_content_length(
        &input.content,
        &input.encoding,
        ctx.byte_limits.max_write_bytes,
    )?;
    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::WriteFile, &parsed)?;
    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let storage = resolved.storage;
    let session_access = ctx
        .validate_session(
            input.session_id.as_deref(),
            &storage.name,
            Some(&resolved.parsed.backend_path),
        )
        .await?;

    if storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            json!({ "storage_name": storage.name, "path": parsed.normalized }),
        ));
    }
    if session_access.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": input.session_id }),
        ));
    }
    enforce_storage_policy(
        &storage,
        &resolved.parsed.backend_path,
        McpOperation::Write,
        input.overwrite,
        false,
    )?;

    // Size-check and decode before building the operator or any side effect,
    // so an oversized or malformed payload never creates parents or reaches
    // the backend. The content String is consumed, not copied.
    let bytes = decode_content(
        std::mem::take(&mut input.content),
        &input.encoding,
        ctx.byte_limits.max_write_bytes,
    )?;

    let op = opendal_adapter::build_operator(&storage, &ctx.registry)?;

    if parsed.backend_path.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let written_bytes = bytes.len() as u64;

    // All requested metadata and write guarantees must be supported before
    // parent creation or opening a backend writer.
    let user_metadata = sanitize_user_metadata(input.user_metadata);
    validate_write_capabilities(
        &op,
        input.overwrite,
        input.content_type.as_deref(),
        user_metadata.is_some(),
    )?;

    if input.overwrite {
        match op.stat(&parsed.backend_path).await {
            Ok(meta) if meta.is_dir() => {
                return Err(err_with_details(
                    McpErrorCode::ERR_IS_A_DIRECTORY,
                    "path is a directory",
                    json!({ "path": parsed.normalized }),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    if input.create_parents {
        if let Some(parent) = parent_path(&parsed.backend_path) {
            let missing =
                missing_directory_paths(&op, &parent, &storage.name, parsed.normalized.as_str())
                    .await?;
            for ancestor in &missing {
                let ancestor_session = ctx
                    .validate_session(input.session_id.as_deref(), &storage.name, Some(ancestor))
                    .await?;
                if ancestor_session.read_only {
                    return Err(err_with_details(
                        McpErrorCode::ERR_SESSION_FORBIDDEN,
                        "session cannot create a parent directory",
                        json!({ "session_id": input.session_id }),
                    ));
                }
                enforce_storage_policy(&storage, ancestor, McpOperation::Mkdir, false, false)?;
            }
            create_dir_chain(&op, &parent, &storage.name, parsed.normalized.as_str()).await?;
        }
    } else if let Some(parent) = parent_path(&parsed.backend_path) {
        match op.stat(&parent).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(err_with_details(
                    McpErrorCode::ERR_PARENT_NOT_FOUND,
                    "parent directory does not exist",
                    json!({ "path": parsed.normalized, "parent": format!("/{}/{}", storage.name, parent) }),
                ));
            }
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {
                return Err(err_with_details(
                    McpErrorCode::ERR_PARENT_NOT_FOUND,
                    "parent directory does not exist",
                    json!({ "path": parsed.normalized, "parent": format!("/{}/{}", storage.name, parent) }),
                ));
            }
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    infimount_core::operations::write_full_with_options(
        &op,
        &parsed.backend_path,
        bytes,
        input.overwrite,
        input.content_type.as_deref(),
        user_metadata,
    )
    .await
    .map_err(|error| match error {
        infimount_core::CoreError::Storage(error)
            if !input.overwrite
                && matches!(
                    error.kind(),
                    opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch
                ) =>
        {
            err_with_details(
                McpErrorCode::ERR_ALREADY_EXISTS,
                "path already exists",
                json!({ "path": parsed.normalized }),
            )
        }
        error => core_error_to_mcp_error(error),
    })?;

    Ok(WriteFileOutput {
        path: parsed.normalized,
        written_bytes,
    })
}

/// Whether the backend can guarantee an atomic create-if-absent write.
pub(crate) fn supports_atomic_no_overwrite(capability: &opendal::Capability) -> bool {
    capability.write_with_if_not_exists
}

pub(crate) fn validate_content_type(content_type: Option<&str>) -> McpResult<()> {
    if content_type.is_some_and(|value| !infimount_core::operations::valid_content_type(value)) {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "content_type must be a valid media type of at most 255 ASCII bytes",
            json!({}),
        ));
    }
    Ok(())
}

pub(crate) fn validate_write_capabilities(
    op: &opendal::Operator,
    overwrite: bool,
    content_type: Option<&str>,
    has_user_metadata: bool,
) -> McpResult<()> {
    if !overwrite && !supports_atomic_no_overwrite(&op.info().capability()) {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            "storage backend cannot guarantee an atomic no-overwrite write",
            json!({}),
        ));
    }
    infimount_core::operations::validate_write_options(
        op,
        overwrite,
        content_type,
        has_user_metadata,
    )
    .map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            "storage backend does not support the requested write metadata",
            json!({}),
        )
    })
}

/// Validate base64 before creating an approval. A fixed buffer avoids retaining
/// a second copy of the payload while the human decides.
pub(crate) fn validate_write_content(
    content: &str,
    encoding: &str,
    max_bytes: u32,
) -> McpResult<()> {
    validate_write_content_length(content, encoding, max_bytes)?;
    if encoding.eq_ignore_ascii_case("base64") {
        use std::io::Read;
        let mut decoder = base64::read::DecoderReader::new(content.as_bytes(), &BASE64_STANDARD);
        let mut chunk = [0_u8; 4096];
        let mut decoded = 0_usize;
        loop {
            let count = decoder.read(&mut chunk).map_err(|_| {
                err_with_details(
                    McpErrorCode::ERR_TEXT_DECODE_FAILED,
                    "content is not valid base64",
                    json!({ "encoding": "base64" }),
                )
            })?;
            if count == 0 {
                break;
            }
            decoded += count;
            if decoded > max_bytes as usize {
                return Err(content_too_large(max_bytes as usize));
            }
        }
    }
    Ok(())
}

fn sanitize_user_metadata(
    user_metadata: Option<HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    let metadata = user_metadata?;
    let sanitized = metadata
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_string();
            if key.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect::<HashMap<_, _>>();

    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// Content encodings `write_file` accepts. `utf-8` writes the text as-is;
/// `base64` (RFC 4648, standard alphabet, padded) writes the decoded bytes so
/// agents can store binary output such as images, spreadsheets or archives.
pub const SUPPORTED_WRITE_ENCODINGS: [&str; 2] = ["utf-8", "base64"];

/// Cheap preflight, also used by the MCP confirmation path before it hashes
/// or queues a write. The actual decoder remains authoritative for validity.
pub(crate) fn validate_write_content_length(
    content: &str,
    encoding: &str,
    max_bytes: u32,
) -> McpResult<()> {
    let max_bytes = max_bytes as usize;
    if encoding.eq_ignore_ascii_case("utf-8") {
        if content.len() > max_bytes {
            return Err(content_too_large(max_bytes));
        }
        return Ok(());
    }
    if encoding.eq_ignore_ascii_case("base64") {
        if content.len() > max_bytes.div_ceil(3) * 4 {
            return Err(content_too_large(max_bytes));
        }
        // For a potentially valid padded payload, account for the last
        // quartet. The encoded-length bound alone permits up to two extra bytes.
        if content.len().is_multiple_of(4) {
            let padding = if content.ends_with("==") {
                2
            } else if content.ends_with('=') {
                1
            } else {
                0
            };
            let decoded_len = (content.len() / 4 * 3).saturating_sub(padding);
            if decoded_len > max_bytes {
                return Err(content_too_large(max_bytes));
            }
        }
        return Ok(());
    }
    Err(err_with_details(
        McpErrorCode::ERR_TEXT_DECODE_FAILED,
        "unsupported content encoding",
        json!({
            "supported_encodings": SUPPORTED_WRITE_ENCODINGS,
            "supported_encoding": "utf-8"
        }),
    ))
}

fn decode_content(content: String, encoding: &str, max_bytes: u32) -> McpResult<Vec<u8>> {
    validate_write_content_length(&content, encoding, max_bytes)?;
    let max_bytes = max_bytes as usize;
    if encoding.eq_ignore_ascii_case("utf-8") {
        return Ok(content.into_bytes());
    }
    let bytes = BASE64_STANDARD.decode(content.as_bytes()).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_TEXT_DECODE_FAILED,
            "content is not valid base64",
            json!({
                "encoding": "base64",
                "hint": "use standard base64 (RFC 4648) with padding"
            }),
        )
    })?;
    // Keep the decoded size check as defense in depth.
    if bytes.len() > max_bytes {
        return Err(content_too_large(max_bytes));
    }
    Ok(bytes)
}

fn content_too_large(max_bytes: usize) -> McpError {
    err_with_details(
        McpErrorCode::ERR_INVALID_PATH,
        "file content exceeds the configured write limit",
        json!({ "max_bytes": max_bytes }),
    )
}
