use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{
    copy_directory, copy_file_chunked, delete_existing_on_operator, ensure_parent_exists_for_copy,
    FsToolsContext,
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

    if dst_resolved.storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", dst_resolved.storage.name),
            json!({ "storage_name": dst_resolved.storage.name, "path": dst_parsed.normalized }),
        ));
    }

    if src_resolved.storage.id == dst_resolved.storage.id
        && src_parsed.backend_path == dst_parsed.backend_path
    {
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
        Some(
            src_op
                .stat(&src_parsed.backend_path)
                .await
                .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?,
        )
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

    let dst_meta = if dst_parsed.backend_path.is_empty() {
        None
    } else {
        match dst_op.stat(&dst_parsed.backend_path).await {
            Ok(meta) => Some(meta),
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => None,
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    };

    let dst_exists = dst_parsed.backend_path.is_empty() || dst_meta.is_some();
    if dst_exists && !input.overwrite {
        return Err(err_with_details(
            McpErrorCode::ERR_ALREADY_EXISTS,
            "destination path already exists",
            json!({ "dst": dst_parsed.normalized }),
        ));
    }

    if dst_exists && input.overwrite && !dst_parsed.backend_path.is_empty() {
        delete_existing_on_operator(
            &dst_op,
            &dst_resolved.storage.name,
            &dst_parsed.backend_path,
            dst_meta.as_ref().map(|meta| meta.is_dir()).unwrap_or(false),
        )
        .await?;
    }

    if src_is_dir {
        copy_directory(
            &src_op,
            &dst_op,
            &src_resolved.storage.name,
            &dst_resolved.storage.name,
            &src_parsed.backend_path,
            &dst_parsed.backend_path,
            src_resolved.storage.id == dst_resolved.storage.id,
        )
        .await?;
    } else {
        ensure_parent_exists_for_copy(
            &dst_op,
            &dst_resolved.storage.name,
            &dst_parsed.backend_path,
        )
        .await?;

        if src_resolved.storage.id == dst_resolved.storage.id {
            src_op
                .copy(&src_parsed.backend_path, &dst_parsed.backend_path)
                .await
                .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
        } else {
            let src_meta = src_meta.expect("file copy must have source metadata");
            copy_file_chunked(
                &src_op,
                &dst_op,
                &src_parsed.backend_path,
                &dst_parsed.backend_path,
                src_meta.content_length(),
            )
            .await?;
        }
    }

    Ok(CopyPathOutput {
        src: src_parsed.normalized,
        dst: dst_parsed.normalized,
        copied: true,
    })
}
