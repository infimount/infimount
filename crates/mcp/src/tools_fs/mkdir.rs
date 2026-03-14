use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{
    create_dir_chain, default_true, normalize_list_prefix, parent_path, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MkdirInput {
    pub path: String,
    #[serde(default = "default_true")]
    pub parents: bool,
    #[serde(default = "default_true")]
    pub exist_ok: bool,
}

#[derive(Debug, Serialize)]
pub struct MkdirOutput {
    pub path: String,
    pub created: bool,
}

pub async fn mkdir(ctx: &FsToolsContext, input: MkdirInput) -> McpResult<MkdirOutput> {
    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::Mkdir, &parsed)?;
    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let storage = resolved.storage;

    if storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            json!({ "storage_name": storage.name, "path": parsed.normalized }),
        ));
    }

    let op = opendal_adapter::build_operator(&storage)?;

    if parsed.backend_path.is_empty() {
        if input.exist_ok {
            return Ok(MkdirOutput {
                path: parsed.normalized,
                created: false,
            });
        }
        return Err(err_with_details(
            McpErrorCode::ERR_ALREADY_EXISTS,
            "path already exists",
            json!({ "path": parsed.normalized }),
        ));
    }

    match op.stat(&parsed.backend_path).await {
        Ok(_) => {
            if input.exist_ok {
                return Ok(MkdirOutput {
                    path: parsed.normalized,
                    created: false,
                });
            }
            return Err(err_with_details(
                McpErrorCode::ERR_ALREADY_EXISTS,
                "path already exists",
                json!({ "path": parsed.normalized }),
            ));
        }
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
        Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
    }

    if !input.parents {
        if let Some(parent) = parent_path(&parsed.backend_path) {
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
    }

    if input.parents {
        create_dir_chain(
            &op,
            &parsed.backend_path,
            &storage.name,
            parsed.normalized.as_str(),
        )
        .await?;
    } else {
        let create_target = normalize_list_prefix(&parsed.backend_path);
        op.create_dir(&create_target)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    }

    Ok(MkdirOutput {
        path: parsed.normalized,
        created: true,
    })
}
