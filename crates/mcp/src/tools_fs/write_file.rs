use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{
    create_dir_chain, default_encoding, default_true, parent_path, FsToolsContext,
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

    if storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            json!({ "storage_name": storage.name, "path": parsed.normalized }),
        ));
    }

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

    let op = opendal_adapter::build_operator(&storage)?;

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

    let bytes = input.content.into_bytes();
    op.write(&parsed.backend_path, bytes.clone())
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;

    Ok(WriteFileOutput {
        path: parsed.normalized,
        written_bytes: bytes.len() as u64,
    })
}
