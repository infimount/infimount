use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{
    backend_path_from_virtual, collect_entries, normalize_list_prefix, path_depth, sort_entries,
    EntryType, FsToolsContext,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePathInput {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
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

    if storage.read_only {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            json!({ "storage_name": storage.name, "path": parsed.normalized }),
        ));
    }

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

    let mut entries = collect_entries(&op, &storage.name, &parsed.backend_path, true).await?;
    sort_entries(&mut entries, true);

    let mut delete_order = entries;
    delete_order.sort_by(|a, b| {
        path_depth(&b.path).cmp(&path_depth(&a.path)).then_with(|| {
            match (a.entry_type, b.entry_type) {
                (EntryType::File, EntryType::Dir) => std::cmp::Ordering::Less,
                (EntryType::Dir, EntryType::File) => std::cmp::Ordering::Greater,
                _ => a.path.cmp(&b.path),
            }
        })
    });

    for entry in delete_order {
        let backend_path = backend_path_from_virtual(&storage.name, &entry.path);
        let delete_target = match entry.entry_type {
            EntryType::File => backend_path,
            EntryType::Dir => normalize_list_prefix(&backend_path),
        };

        if delete_target.is_empty() {
            continue;
        }

        match op.delete(&delete_target).await {
            Ok(()) => {}
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    if !parsed.backend_path.is_empty() {
        let dir_target = normalize_list_prefix(&parsed.backend_path);
        match op.delete(&dir_target).await {
            Ok(()) => {}
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    Ok(DeletePathOutput {
        path: parsed.normalized,
        deleted: true,
    })
}
