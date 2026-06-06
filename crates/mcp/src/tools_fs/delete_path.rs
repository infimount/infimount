use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{
    collect_entries_with_policy, enforce_storage_policy,
    DeniedDescendantBehavior, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePathInput {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub confirmation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeletePathOutput {
    pub path: String,
    pub deleted: bool,
}

pub async fn delete_path(
    ctx: &FsToolsContext,
    input: DeletePathInput,
) -> McpResult<DeletePathOutput> {
    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::DeletePath, &parsed)?;
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
        McpOperation::Delete,
        false,
        false,
    )?;

    let op = opendal_adapter::build_operator(&storage)?;
    let target_meta = if parsed.backend_path.is_empty() {
        None
    } else {
        Some(
            op.stat(&parsed.backend_path)
                .await
                .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?,
        )
    };

    let is_dir = parsed.backend_path.is_empty()
        || target_meta
            .as_ref()
            .map(|meta| meta.is_dir())
            .unwrap_or(false);

    if !is_dir {
        op.delete(&parsed.backend_path)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
        return Ok(DeletePathOutput {
            path: parsed.normalized,
            deleted: true,
        });
    }

    if !input.recursive {
        return Err(err_with_details(
            McpErrorCode::ERR_NOT_EMPTY_OR_DIR,
            "directory deletion requires recursive=true",
            json!({ "path": parsed.normalized }),
        ));
    }

    // MCP-specific pre-flight check for descendant policies
    collect_entries_with_policy(
        &op,
        &storage,
        &parsed.backend_path,
        true,
        McpOperation::Delete,
        DeniedDescendantBehavior::Fail,
    )
    .await?;

    infimount_core::operations::delete(&op, &parsed.backend_path)
        .await
        .map_err(super::common::core_error_to_mcp_error)?;

    Ok(DeletePathOutput {
        path: parsed.normalized,
        deleted: true,
    })
}
