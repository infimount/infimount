use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_core_error, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{default_limit, enforce_storage_policy, FsToolsContext};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListVersionsInput {
    pub path: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListVersionsOutput {
    pub path: String,
    pub versions: Vec<VersionEntry>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

pub async fn list_versions(
    ctx: &FsToolsContext,
    input: ListVersionsInput,
) -> McpResult<ListVersionsOutput> {
    if input.limit == 0 || input.limit > 1000 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "limit must be between 1 and 1000",
            json!({ "limit": input.limit }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ListVersions, &parsed)?;

    if parsed.is_root {
        return Err(err_with_details(
            McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED,
            "cannot list versions at root",
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
        McpOperation::ListVersions,
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
    if !capabilities.list_with_versions {
        return Err(err_with_details(
            McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED,
            "version listing not supported for this storage backend",
            json!({
                "path": parsed.normalized,
                "storage": resolved.storage.name
            }),
        ));
    }

    let meta = op
        .stat(&parsed.backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND))?;

    if meta.is_dir() {
        return Err(err_with_details(
            McpErrorCode::ERR_IS_A_DIRECTORY,
            "cannot list versions for a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let result = infimount_core::operations::list_file_versions_page(
        &op,
        &resolved.storage.id,
        &parsed.backend_path,
        input.limit,
        input.cursor.as_deref(),
        resolved.storage.revision,
    )
    .await
    .map_err(|e| match &e {
        infimount_core::CoreError::Config(_) => err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid version cursor or path",
            json!({ "path": parsed.normalized }),
        ),
        _ => map_core_error(&e),
    })?;

    Ok(ListVersionsOutput {
        path: parsed.normalized,
        versions: result
            .versions
            .into_iter()
            .map(|v| VersionEntry {
                version: v.version,
                size_bytes: v.size_bytes,
                modified_at: v.modified_at,
                etag: v.etag,
            })
            .collect(),
        next_cursor: result.next_cursor,
        truncated: result.truncated,
    })
}
