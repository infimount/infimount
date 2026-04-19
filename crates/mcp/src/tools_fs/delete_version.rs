use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::FsToolsContext;

#[derive(Debug, Deserialize, Default)]
pub struct DeleteVersionInput {
    pub path: String,
    pub version: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteVersionOutput {
    pub path: String,
    pub version: String,
    pub deleted: bool,
}

pub async fn delete_version(
    ctx: &FsToolsContext,
    input: DeleteVersionInput,
) -> McpResult<DeleteVersionOutput> {
    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::DeleteVersion, &parsed)?;

    if parsed.backend_path.is_empty() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "path is a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
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
    if !capabilities.delete_with_version {
        return Err(err_with_details(
            McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED,
            "version deletion not supported for this storage backend",
            json!({
                "path": parsed.normalized,
                "storage": resolved.storage.name
            }),
        ));
    }

    let session_access = ctx
        .validate_session(
            input.session_id.as_deref(),
            &resolved.storage.name,
            Some(&resolved.parsed.backend_path),
        )
        .await?;
    if session_access.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": input.session_id }),
        ));
    }

    op.delete_with(&parsed.backend_path)
        .version(&input.version)
        .await
        .map_err(|e: opendal::Error| {
            if e.kind() == opendal::ErrorKind::NotFound {
                return map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND);
            }
            map_opendal_error(&e, McpErrorCode::ERR_INTERNAL)
        })?;

    Ok(DeleteVersionOutput {
        path: parsed.normalized,
        version: input.version,
        deleted: true,
    })
}
