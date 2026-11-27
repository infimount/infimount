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
    create_dir_chain, default_encoding, default_true, enforce_storage_policy,
    missing_directory_paths, parent_path, FsToolsContext,
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
}

#[derive(Debug, Serialize)]
pub struct WriteFileOutput {
    pub path: String,
    pub written_bytes: u64,
}

/// Maximum number of bytes an MCP `write_file` request may write (after decoding).
pub const MAX_MCP_WRITE_BYTES: usize = 4 * 1024 * 1024;

/// Longest padded base64 text whose decoded bytes can fit in
/// [`MAX_MCP_WRITE_BYTES`]. Shared by the runtime pre-check and the schema
/// `maxLength`, so the two cannot drift.
pub const MAX_MCP_WRITE_BASE64_LEN: usize = MAX_MCP_WRITE_BYTES.div_ceil(3) * 4;

pub async fn write_file(
    ctx: &FsToolsContext,
    mut input: WriteFileInput,
) -> McpResult<WriteFileOutput> {
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
        &parsed.normalized,
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

    // Capability gates must run before any side effect: an unsupported
    // no-overwrite write must not leave freshly created parent directories
    // behind when it is rejected.
    if !input.overwrite && !supports_atomic_no_overwrite(&op.info().capability()) {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            "storage backend cannot guarantee an atomic no-overwrite write",
            json!({ "path": parsed.normalized, "storage_name": storage.name }),
        ));
    }

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

    let user_metadata = sanitize_user_metadata(input.user_metadata);
    if user_metadata.is_some() && !op.info().capability().write_with_user_metadata {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            "storage backend does not support user metadata writes",
            json!({ "path": parsed.normalized, "storage_name": storage.name }),
        ));
    }

    if !input.overwrite {
        let write = op
            .write_with(&parsed.backend_path, bytes)
            .if_not_exists(true);
        let write = match user_metadata {
            Some(metadata) => write.user_metadata(metadata),
            None => write,
        };
        write.await.map_err(|e| match e.kind() {
            opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch => {
                err_with_details(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    "path already exists",
                    json!({ "path": parsed.normalized }),
                )
            }
            _ => map_opendal_error(&e, McpErrorCode::ERR_INTERNAL),
        })?;
        return Ok(WriteFileOutput {
            path: parsed.normalized,
            written_bytes,
        });
    }

    if let Some(metadata) = user_metadata {
        op.write_with(&parsed.backend_path, bytes)
            .user_metadata(metadata)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    } else {
        op.write(&parsed.backend_path, bytes)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    }

    Ok(WriteFileOutput {
        path: parsed.normalized,
        written_bytes,
    })
}

/// Whether the backend can guarantee an atomic create-if-absent write.
pub(crate) fn supports_atomic_no_overwrite(capability: &opendal::Capability) -> bool {
    capability.write_with_if_not_exists
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

fn decode_content(content: String, encoding: &str, path: &str) -> McpResult<Vec<u8>> {
    if encoding.eq_ignore_ascii_case("utf-8") {
        // UTF-8 bytes are the content itself: check the length, then take
        // ownership without copying.
        if content.len() > MAX_MCP_WRITE_BYTES {
            return Err(content_too_large(path));
        }
        return Ok(content.into_bytes());
    }
    if encoding.eq_ignore_ascii_case("base64") {
        // Bound the decoder's input first; base64 never decodes to more bytes
        // than its text length, so this caps the allocation too.
        if content.len() > MAX_MCP_WRITE_BASE64_LEN {
            return Err(content_too_large(path));
        }
        let bytes = BASE64_STANDARD.decode(content.as_bytes()).map_err(|err| {
            err_with_details(
                McpErrorCode::ERR_TEXT_DECODE_FAILED,
                "content is not valid base64",
                json!({
                    "path": path,
                    "encoding": "base64",
                    "reason": err.to_string(),
                    "hint": "use standard base64 (RFC 4648) with padding"
                }),
            )
        })?;
        // A maximal-length payload can still decode to up to 2 bytes over the
        // cap, so the decoded size is checked as well.
        if bytes.len() > MAX_MCP_WRITE_BYTES {
            return Err(content_too_large(path));
        }
        return Ok(bytes);
    }
    Err(err_with_details(
        McpErrorCode::ERR_TEXT_DECODE_FAILED,
        "unsupported content encoding",
        json!({
            "path": path,
            "encoding": encoding,
            "supported_encoding": "utf-8",
            "supported_encodings": SUPPORTED_WRITE_ENCODINGS
        }),
    ))
}

fn content_too_large(path: &str) -> McpError {
    err_with_details(
        McpErrorCode::ERR_INVALID_PATH,
        "file content exceeds the 4 MiB write limit",
        json!({ "path": path, "max_bytes": MAX_MCP_WRITE_BYTES }),
    )
}
