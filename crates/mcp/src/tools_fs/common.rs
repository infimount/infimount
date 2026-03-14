use futures::io::AsyncWriteExt;
use futures::TryStreamExt;
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::registry::StorageRegistry;

pub const COPY_CHUNK_SIZE: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct FsToolsContext {
    pub registry: StorageRegistry,
}

#[derive(Debug, Serialize, Clone)]
pub struct ListDirEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    File,
    Dir,
}

pub(super) fn default_limit() -> u32 {
    200
}

pub(super) fn default_read_max_bytes() -> u32 {
    262_144
}

pub(super) fn default_as_text() -> bool {
    true
}

pub(super) fn default_encoding() -> String {
    "utf-8".to_string()
}

pub(super) fn default_true() -> bool {
    true
}

pub(super) async fn collect_entries(
    op: &opendal::Operator,
    storage_name: &str,
    backend_path: &str,
    recursive: bool,
) -> McpResult<Vec<ListDirEntry>> {
    let mut out = Vec::new();
    let mut stack = vec![normalize_list_prefix(backend_path)];
    let mut visited = HashSet::new();

    while let Some(current_prefix) = stack.pop() {
        if recursive && !visited.insert(current_prefix.clone()) {
            continue;
        }

        let mut lister = if current_prefix.is_empty() {
            match op.lister("").await {
                Ok(l) => l,
                Err(e) if e.kind() == opendal::ErrorKind::NotFound => op
                    .lister("/")
                    .await
                    .map_err(|err| map_opendal_error(&err, McpErrorCode::ERR_PATH_NOT_FOUND))?,
                Err(e) => return Err(map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND)),
            }
        } else {
            op.lister(&current_prefix)
                .await
                .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_PATH_NOT_FOUND))?
        };

        while let Some(obj) = lister
            .try_next()
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?
        {
            let object_path = obj.path().to_string();
            let meta = op
                .stat(&object_path)
                .await
                .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;

            let is_dir = meta.is_dir();
            let normalized_object = object_path.trim_end_matches('/').to_string();
            if normalized_object.is_empty() {
                continue;
            }
            if normalized_object == current_prefix.trim_end_matches('/') {
                continue;
            }
            let name = extract_name(&normalized_object);
            let full_path = format!("/{storage_name}/{}", normalized_object);

            out.push(ListDirEntry {
                name,
                path: full_path,
                entry_type: if is_dir {
                    EntryType::Dir
                } else {
                    EntryType::File
                },
                size_bytes: if is_dir {
                    None
                } else {
                    Some(meta.content_length())
                },
                modified_at: meta.last_modified().map(|dt| dt.to_string()),
                etag: meta.etag().map(|s| s.to_string()),
            });

            if recursive && is_dir {
                stack.push(normalize_list_prefix(&normalized_object));
            }
        }

        if !recursive {
            break;
        }
    }

    Ok(out)
}

pub(super) fn normalize_list_prefix(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

pub(super) fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

pub(super) async fn create_dir_chain(
    op: &opendal::Operator,
    backend_path: &str,
    storage_name: &str,
    full_path: &str,
) -> McpResult<()> {
    let trimmed = backend_path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(());
    }

    let mut current = String::new();
    for segment in trimmed.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);

        match op.stat(&current).await {
            Ok(meta) if meta.is_dir() => continue,
            Ok(_) => {
                return Err(err_with_details(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    "path already exists as a file",
                    json!({
                        "path": full_path,
                        "intermediate_path": format!("/{}/{}", storage_name, current)
                    }),
                ));
            }
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {
                let dir_path = normalize_list_prefix(&current);
                op.create_dir(&dir_path)
                    .await
                    .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
            }
            Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }
    }

    Ok(())
}

pub(super) async fn ensure_parent_exists_for_copy(
    op: &opendal::Operator,
    storage_name: &str,
    backend_path: &str,
) -> McpResult<()> {
    if let Some(parent) = parent_path(backend_path) {
        match op.stat(&parent).await {
            Ok(meta) if meta.is_dir() => Ok(()),
            Ok(_) => Err(err_with_details(
                McpErrorCode::ERR_PARENT_NOT_FOUND,
                "parent directory does not exist",
                json!({ "parent": format!("/{}/{}", storage_name, parent) }),
            )),
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => Err(err_with_details(
                McpErrorCode::ERR_PARENT_NOT_FOUND,
                "parent directory does not exist",
                json!({ "parent": format!("/{}/{}", storage_name, parent) }),
            )),
            Err(err) => Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
        }?
    }

    Ok(())
}

pub(super) async fn copy_file_chunked(
    src_op: &opendal::Operator,
    dst_op: &opendal::Operator,
    src_backend_path: &str,
    dst_backend_path: &str,
    size: u64,
) -> McpResult<()> {
    let mut writer = dst_op
        .writer(dst_backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?
        .into_futures_async_write();

    let mut offset = 0_u64;
    while offset < size {
        let end = (offset + COPY_CHUNK_SIZE).min(size);
        let chunk = src_op
            .read_with(src_backend_path)
            .range(offset..end)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?
            .to_vec();
        writer.write_all(&chunk).await.map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to write destination file",
                json!({ "io_error": e.to_string() }),
            )
        })?;
        offset = end;
    }

    writer.close().await.map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to finalize destination file",
            json!({ "io_error": e.to_string() }),
        )
    })?;

    Ok(())
}

pub(super) async fn delete_existing_on_operator(
    op: &opendal::Operator,
    storage_name: &str,
    backend_path: &str,
    is_dir: bool,
) -> McpResult<()> {
    if !is_dir {
        op.delete(backend_path)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
        return Ok(());
    }

    let mut entries = collect_entries(op, storage_name, backend_path, true).await?;
    sort_entries(&mut entries, true);
    entries.sort_by(|a, b| {
        path_depth(&b.path).cmp(&path_depth(&a.path)).then_with(|| {
            match (a.entry_type, b.entry_type) {
                (EntryType::File, EntryType::Dir) => std::cmp::Ordering::Less,
                (EntryType::Dir, EntryType::File) => std::cmp::Ordering::Greater,
                _ => a.path.cmp(&b.path),
            }
        })
    });

    for entry in entries {
        let child_backend = backend_path_from_virtual(storage_name, &entry.path);
        let delete_target = match entry.entry_type {
            EntryType::File => child_backend,
            EntryType::Dir => normalize_list_prefix(&child_backend),
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

    let dir_target = normalize_list_prefix(backend_path);
    match op.delete(&dir_target).await {
        Ok(()) => {}
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
        Err(err) => return Err(map_opendal_error(&err, McpErrorCode::ERR_INTERNAL)),
    }

    Ok(())
}

pub(super) async fn copy_directory(
    src_op: &opendal::Operator,
    dst_op: &opendal::Operator,
    src_storage_name: &str,
    dst_storage_name: &str,
    src_backend_root: &str,
    dst_backend_root: &str,
    same_storage: bool,
) -> McpResult<()> {
    if !dst_backend_root.is_empty() {
        create_dir_chain(dst_op, dst_backend_root, dst_storage_name, dst_backend_root).await?;
    }

    let mut entries = collect_entries(src_op, src_storage_name, src_backend_root, true).await?;
    sort_entries(&mut entries, true);

    for entry in entries {
        let src_backend = backend_path_from_virtual(src_storage_name, &entry.path);
        let relative = relative_backend_path(src_backend_root, &src_backend);
        let dst_backend = join_backend_path(dst_backend_root, &relative);

        match entry.entry_type {
            EntryType::Dir => {
                create_dir_chain(dst_op, &dst_backend, dst_storage_name, &entry.path).await?;
            }
            EntryType::File => {
                ensure_parent_exists_for_copy(dst_op, dst_storage_name, &dst_backend).await?;
                if same_storage {
                    src_op
                        .copy(&src_backend, &dst_backend)
                        .await
                        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
                } else {
                    let meta = src_op
                        .stat(&src_backend)
                        .await
                        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
                    copy_file_chunked(
                        src_op,
                        dst_op,
                        &src_backend,
                        &dst_backend,
                        meta.content_length(),
                    )
                    .await?;
                }
            }
        }
    }

    Ok(())
}

pub(super) fn backend_path_from_virtual(storage_name: &str, full_path: &str) -> String {
    let storage_root = format!("/{storage_name}");
    if full_path == storage_root {
        return String::new();
    }

    full_path
        .strip_prefix(&(storage_root + "/"))
        .unwrap_or("")
        .to_string()
}

pub(super) fn path_depth(path: &str) -> usize {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

pub(super) fn extract_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

pub(super) fn sort_entries(entries: &mut [ListDirEntry], recursive: bool) {
    if recursive {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        return;
    }

    entries.sort_by(|a, b| match (a.entry_type, b.entry_type) {
        (EntryType::Dir, EntryType::File) => std::cmp::Ordering::Less,
        (EntryType::File, EntryType::Dir) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
}

pub(super) fn relative_backend_path(root: &str, full: &str) -> String {
    if root.is_empty() {
        return full.to_string();
    }

    full.strip_prefix(&(root.to_string() + "/"))
        .unwrap_or(full)
        .to_string()
}

pub(super) fn join_backend_path(base: &str, relative: &str) -> String {
    if base.is_empty() {
        return relative.to_string();
    }
    if relative.is_empty() {
        return base.to_string();
    }
    format!("{base}/{relative}")
}
