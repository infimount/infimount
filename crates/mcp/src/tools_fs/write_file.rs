use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
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

pub async fn write_file(ctx: &FsToolsContext, input: WriteFileInput) -> McpResult<WriteFileOutput> {
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

    if !input.encoding.eq_ignore_ascii_case("utf-8") {
        return Err(err_with_details(
            McpErrorCode::ERR_TEXT_DECODE_FAILED,
            "failed to encode file content as text",
            json!({
                "path": parsed.normalized,
                "encoding": input.encoding,
                "supported_encoding": "utf-8"
            }),
        ));
    }

    let op = opendal_adapter::build_operator(&storage, &ctx.registry)?;

    if parsed.backend_path.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    match op.stat(&parsed.backend_path).await {
        Ok(meta) => {
            if meta.is_dir() {
                return Err(err_with_details(
                    McpErrorCode::ERR_IS_A_DIRECTORY,
                    "path is a directory",
                    json!({ "path": parsed.normalized }),
                ));
            }
            if !input.overwrite {
                return Err(err_with_details(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    "path already exists",
                    json!({ "path": parsed.normalized }),
                ));
            }
        }
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
        Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
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

    let bytes = input.content.into_bytes();
    if let Some(metadata) = user_metadata {
        op.write_with(&parsed.backend_path, bytes.clone())
            .user_metadata(metadata)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    } else {
        op.write(&parsed.backend_path, bytes.clone())
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    }

    Ok(WriteFileOutput {
        path: parsed.normalized,
        written_bytes: bytes.len() as u64,
    })
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
