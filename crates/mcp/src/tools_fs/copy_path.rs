use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use infimount_core::operations::{TransferConflictPolicy, TransferOperation};

use super::common::{
    collect_entries_with_policy, core_error_to_mcp_error, enforce_storage_policy, FsToolsContext,
    DeniedDescendantBehavior,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyPathInput {
    pub src: String,
    pub dst: String,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub confirmation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CopyPathOutput {
    pub src: String,
    pub dst: String,
    pub copied: bool,
}

pub async fn copy_path(ctx: &FsToolsContext, input: CopyPathInput) -> McpResult<CopyPathOutput> {
    let src_parsed = parse_mcp_path(&input.src)?;
    enforce_root_operation(FsOp::CopyPath, &src_parsed)?;
    let dst_parsed = parse_mcp_path(&input.dst)?;
    enforce_root_operation(FsOp::CopyPath, &dst_parsed)?;

    let src_resolved = resolve_storage_path(&ctx.registry, &src_parsed.normalized)?;
    let dst_resolved = resolve_storage_path(&ctx.registry, &dst_parsed.normalized)?;
    ctx.validate_session(
        input.session_id.as_deref(),
        &src_resolved.storage.name,
        Some(&src_resolved.parsed.backend_path),
    )
    .await?;
    let dst_session_access = ctx
        .validate_session(
            input.session_id.as_deref(),
            &dst_resolved.storage.name,
            Some(&dst_resolved.parsed.backend_path),
        )
        .await?;

    if dst_resolved.storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", dst_resolved.storage.name),
            json!({ "storage_name": dst_resolved.storage.name, "path": dst_parsed.normalized }),
        ));
    }
    if dst_session_access.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": input.session_id }),
        ));
    }
    let same_storage = src_resolved.storage.id == dst_resolved.storage.id;
    enforce_storage_policy(
        &src_resolved.storage,
        &src_resolved.parsed.backend_path,
        McpOperation::Read,
        false,
        false,
    )?;
    enforce_storage_policy(
        &dst_resolved.storage,
        &dst_resolved.parsed.backend_path,
        McpOperation::Copy,
        input.overwrite,
        !same_storage,
    )?;

    if same_storage && src_parsed.backend_path == dst_parsed.backend_path {
        return Err(err_with_details(
            McpErrorCode::ERR_ALREADY_EXISTS,
            "source and destination are the same path",
            json!({ "src": src_parsed.normalized, "dst": dst_parsed.normalized }),
        ));
    }

    let src_op = opendal_adapter::build_operator(&src_resolved.storage)?;
    let dst_op = opendal_adapter::build_operator(&dst_resolved.storage)?;

    let src_meta = if src_parsed.backend_path.is_empty() {
        None
    } else {
        match src_op.stat(&src_parsed.backend_path).await {
            Ok(meta) => Some(meta),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => {
                return Err(err_with_details(
                    McpErrorCode::ERR_PATH_NOT_FOUND,
                    "source path not found",
                    json!({ "path": src_parsed.normalized }),
                ));
            }
            Err(e) => return Err(map_opendal_error(&e, McpErrorCode::ERR_INTERNAL)),
        }
    };
    let src_is_dir = src_parsed.backend_path.is_empty()
        || src_meta.as_ref().map(|meta| meta.is_dir()).unwrap_or(false);

    if src_is_dir && !input.recursive {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "source path is a directory",
            json!({ "src": src_parsed.normalized }),
        ));
    }

    if src_is_dir {
        // MCP-specific pre-flight check for descendant policies
        collect_entries_with_policy(
            &src_op,
            &src_resolved.storage,
            &src_parsed.backend_path,
            true,
            McpOperation::Read,
            DeniedDescendantBehavior::Fail,
        )
        .await?;
    }

    infimount_core::operations::transfer_path(
        &src_op,
        &dst_op,
        &src_parsed.backend_path,
        &dst_parsed.backend_path,
        TransferOperation::Copy,
        same_storage,
        if input.overwrite {
            TransferConflictPolicy::Overwrite
        } else {
            TransferConflictPolicy::Fail
        },
    )
    .await
    .map_err(core_error_to_mcp_error)?;

    Ok(CopyPathOutput {
        src: src_parsed.normalized,
        dst: dst_parsed.normalized,
        copied: true,
    })
}
