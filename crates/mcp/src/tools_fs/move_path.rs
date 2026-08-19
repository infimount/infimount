use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;
use infimount_core::operations::{TransferConflictPolicy, TransferOperation};

use super::common::{
    collect_entries_with_policy, core_error_to_mcp_error, enforce_storage_policy,
    DeniedDescendantBehavior, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovePathInput {
    pub src: String,
    pub dst: String,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub confirmation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MovePathOutput {
    pub src: String,
    pub dst: String,
    pub moved: bool,
}

pub async fn move_path(ctx: &FsToolsContext, input: MovePathInput) -> McpResult<MovePathOutput> {
    let src_parsed = parse_mcp_path(&input.src)?;
    enforce_root_operation(FsOp::MovePath, &src_parsed)?;
    let dst_parsed = parse_mcp_path(&input.dst)?;
    enforce_root_operation(FsOp::MovePath, &dst_parsed)?;

    let src_resolved = resolve_storage_path(&ctx.registry, &src_parsed.normalized)?;
    let dst_resolved = resolve_storage_path(&ctx.registry, &dst_parsed.normalized)?;
    let src_session_access = ctx
        .validate_session(
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

    if src_resolved.storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", src_resolved.storage.name),
            json!({ "storage_name": src_resolved.storage.name, "path": src_parsed.normalized }),
        ));
    }
    if dst_resolved.storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", dst_resolved.storage.name),
            json!({ "storage_name": dst_resolved.storage.name, "path": dst_parsed.normalized }),
        ));
    }
    if src_session_access.read_only || dst_session_access.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": input.session_id }),
        ));
    }
    let same_storage = src_resolved.storage.id == dst_resolved.storage.id;
    let namespace_relation = crate::storage_namespace::transfer_namespace_relation(
        &src_resolved.storage,
        &src_parsed.backend_path,
        &dst_resolved.storage,
        &dst_parsed.backend_path,
    )?;
    if crate::storage_namespace::transfer_has_namespace_conflict(&namespace_relation) {
        return Err(err_with_details(
            McpErrorCode::ERR_TRANSFER_NAMESPACE_CONFLICT,
            "move destination overlaps the source namespace",
            json!({ "src": src_parsed.normalized, "dst": dst_parsed.normalized }),
        ));
    }
    enforce_storage_policy(
        &src_resolved.storage,
        &src_resolved.parsed.backend_path,
        McpOperation::Move,
        false,
        !same_storage,
    )?;
    enforce_storage_policy(
        &dst_resolved.storage,
        &dst_resolved.parsed.backend_path,
        McpOperation::Move,
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

    let src_op = opendal_adapter::build_operator(&src_resolved.storage, &ctx.registry)?;
    let dst_op = opendal_adapter::build_operator(&dst_resolved.storage, &ctx.registry)?;

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
    if src_is_dir {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "source path is a directory",
            json!({ "src": src_parsed.normalized }),
        ));
    }

    let dst_meta = if dst_parsed.backend_path.is_empty() {
        None
    } else {
        match dst_op.stat(&dst_parsed.backend_path).await {
            Ok(meta) => Some(meta),
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => None,
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    };

    if let Some(meta) = &dst_meta {
        if input.overwrite {
            enforce_storage_policy(
                &dst_resolved.storage,
                &dst_resolved.parsed.backend_path,
                McpOperation::Delete,
                false,
                false,
            )?;
            if meta.is_dir() {
                collect_entries_with_policy(
                    &dst_op,
                    &dst_resolved.storage,
                    &dst_parsed.backend_path,
                    true,
                    McpOperation::Delete,
                    DeniedDescendantBehavior::Fail,
                )
                .await?;
            }
        }
    }

    ensure_parent_exists(
        &dst_op,
        &dst_resolved.storage.name,
        &dst_parsed.backend_path,
    )
    .await?;

    infimount_core::operations::transfer_path(
        &src_op,
        &dst_op,
        &src_parsed.backend_path,
        &dst_parsed.backend_path,
        TransferOperation::Move,
        namespace_relation.same_operator_scope,
        if input.overwrite {
            TransferConflictPolicy::Overwrite
        } else {
            TransferConflictPolicy::Fail
        },
    )
    .await
    .map_err(core_error_to_mcp_error)?;

    Ok(MovePathOutput {
        src: src_parsed.normalized,
        dst: dst_parsed.normalized,
        moved: true,
    })
}

async fn ensure_parent_exists(
    op: &opendal::Operator,
    storage_name: &str,
    backend_path: &str,
) -> McpResult<()> {
    let Some(parent) = parent_path(backend_path) else {
        return Ok(());
    };

    match op.stat(&parent).await {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(err_with_details(
            McpErrorCode::ERR_PARENT_NOT_FOUND,
            "parent directory does not exist",
            json!({ "parent": format!("/{storage_name}/{parent}") }),
        )),
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => Err(err_with_details(
            McpErrorCode::ERR_PARENT_NOT_FOUND,
            "parent directory does not exist",
            json!({ "parent": format!("/{storage_name}/{parent}") }),
        )),
        Err(err) => Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
    }
}

fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}
