use base64::Engine;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use futures::{StreamExt, TryStreamExt};
use opendal::{ErrorKind, Metadata, Operator};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::fs;

use crate::models::{
    Entry, ListEntriesPage, ReadFileRangeResult, Result, MAX_LIST_LIMIT, MAX_RECURSIVE_ITEMS,
};
use crate::util::extract_filename;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferOperation {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferConflictPolicy {
    /// Fail fast if any destination exists (no partial transfer).
    Fail,
    /// Replace destination objects when they already exist.
    Overwrite,
    /// Skip entries whose destinations already exist.
    Skip,
    /// Keep both by generating a non-conflicting destination name.
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlan {
    pub operation: TransferOperation,
    pub conflict_policy: TransferConflictPolicy,
    pub entries: Vec<TransferPlanEntry>,
    pub summary: TransferPlanSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlanEntry {
    pub source_path: String,
    pub destination_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub action: TransferPlanAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferPlanAction {
    Create,
    Overwrite,
    Skip,
    Rename,
    Noop,
    Conflict,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlanSummary {
    pub create: u64,
    pub overwrite: u64,
    pub skip: u64,
    pub rename: u64,
    pub noop: u64,
    pub conflict: u64,
    pub total_items: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgress {
    pub completed_items: u64,
    pub total_items: u64,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub current_path: String,
}

#[derive(Debug, Clone)]
struct TransferProgressState {
    completed_items: u64,
    total_items: u64,
    bytes_transferred: u64,
    total_bytes: u64,
}

fn normalize_opendal_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return String::new();
    }
    trimmed.trim_start_matches('/').to_string()
}

fn normalize_list_path(path: &str) -> String {
    let mut p = normalize_opendal_path(path);
    if !p.is_empty() && !p.ends_with('/') {
        p.push('/');
    }
    p
}

/// List entries at the given path using the provided operator.
pub async fn list_entries(op: &Operator, path: &str) -> Result<Vec<Entry>> {
    list_entries_with_filter(op, path, |_| Ok(true)).await
}

pub async fn list_entries_with_filter<F>(op: &Operator, path: &str, filter: F) -> Result<Vec<Entry>>
where
    F: Fn(&str) -> Result<bool>,
{
    let p = normalize_list_path(path);
    let mut lister = if p.is_empty() {
        match op.lister("").await {
            Ok(l) => l,
            Err(e) if e.kind() == ErrorKind::NotFound => op.lister("/").await?,
            Err(e) => return Err(e.into()),
        }
    } else {
        op.lister(&p).await?
    };
    let mut out = Vec::new();

    while let Some(obj) = lister.try_next().await? {
        let full_path = obj.path().to_string();
        let normalized_path = full_path.trim_end_matches('/');

        if normalized_path.is_empty() {
            continue;
        }

        if !filter(normalized_path)? {
            continue;
        }

        let name = extract_filename(&full_path);

        // Use op.stat on the full path to ensure we get full metadata.
        // If the entry no longer exists (e.g., broken symlink), keep the
        // entry but leave size/modified blank instead of failing or skipping.
        let (is_dir, size, modified_at, etag) = match op.stat(&full_path).await {
            Ok(meta) => (
                meta.is_dir(),
                meta.content_length(),
                meta.last_modified().map(|dt| dt.to_string()),
                meta.etag().map(|s| s.to_string()),
            ),
            Err(e) if e.kind() == ErrorKind::NotFound => (false, 0, None, None),
            Err(e) => return Err(e.into()),
        };

        let entry = Entry {
            path: full_path,
            name,
            is_dir,
            size,
            modified_at,
            etag,
        };

        out.push(entry);
    }

    Ok(out)
}

/// Recursively list entries below the given path using the provided operator.
pub async fn list_entries_recursive(op: &Operator, path: &str) -> Result<Vec<Entry>> {
    list_entries_recursive_with_filter(op, path, |_| Ok(true)).await
}

pub async fn list_entries_recursive_with_filter<F>(
    op: &Operator,
    path: &str,
    filter: F,
) -> Result<Vec<Entry>>
where
    F: Fn(&str) -> Result<bool>,
{
    let root = normalize_list_path(path);
    let mut out = Vec::new();
    let mut stack = vec![root];

    while let Some(base) = stack.pop() {
        let mut lister = op.lister(&base).await?;
        while let Some(obj) = lister.try_next().await? {
            let full_path = obj.path().to_string();
            let name = extract_filename(&full_path);
            if full_path.is_empty() || name == "." || is_current_dir_marker(&base, &full_path) {
                continue;
            }

            let normalized_path = full_path.trim_end_matches('/');
            if !filter(normalized_path)? {
                continue;
            }

            let meta = op.stat(&full_path).await?;
            let is_dir = meta.is_dir();
            let entry_path = if is_dir {
                ensure_dir_path(&full_path)
            } else {
                full_path.clone()
            };

            out.push(Entry {
                path: entry_path,
                name,
                is_dir,
                size: if is_dir { 0 } else { meta.content_length() },
                modified_at: meta.last_modified().map(|dt| dt.to_string()),
                etag: meta.etag().map(|s| s.to_string()),
            });
            if out.len() > MAX_RECURSIVE_ITEMS as usize {
                out.sort_by(|a, b| a.path.cmp(&b.path));
                return Ok(out);
            }

            if is_dir {
                stack.push(ensure_dir_path(&full_path));
            }
        }
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Stat a single entry.
pub async fn stat_entry(op: &Operator, path: &str) -> Result<Entry> {
    let p = normalize_opendal_path(path);
    let meta = op.stat(&p).await?;
    let full_path = p.to_string();
    let name = extract_filename(&full_path);

    Ok(Entry {
        path: full_path,
        name,
        is_dir: meta.is_dir(),
        size: meta.content_length(),
        modified_at: meta.last_modified().map(|dt| dt.to_string()),
        etag: meta.etag().map(|s| s.to_string()),
    })
}

/// Read the full contents of a file.
pub async fn read_full(op: &Operator, path: &str) -> Result<Vec<u8>> {
    let p = normalize_opendal_path(path);
    let data = op.read(&p).await?;
    Ok(data.to_vec())
}

/// Read a range of a file. Respects max_bytes limits.
/// Read a range of a file. Respects max_bytes limits.
pub async fn read_file_range(
    op: &Operator,
    path: &str,
    offset: u64,
    max_bytes: u64,
) -> Result<ReadFileRangeResult> {
    let p = normalize_opendal_path(path);
    let meta = op.stat(&p).await?;
    let total_size = meta.content_length();

    let actual_max = if max_bytes == 0 {
        crate::models::DEFAULT_PREVIEW_MAX
    } else if max_bytes > crate::models::MAX_READ_RANGE_BYTES {
        return Err(crate::models::CoreError::Config(format!(
            "range exceeds maximum of {} bytes",
            crate::models::MAX_READ_RANGE_BYTES
        )));
    } else {
        max_bytes
    };
    if offset > total_size {
        return Err(crate::models::CoreError::Config(
            "range offset exceeds file size".to_string(),
        ));
    }
    let end = offset.saturating_add(actual_max).min(total_size);
    let actual_bytes = end.saturating_sub(offset);

    if actual_bytes == 0 {
        return Ok(ReadFileRangeResult {
            total_size,
            offset,
            bytes: Vec::new(),
            truncated: false,
        });
    }

    use futures::AsyncReadExt;
    let reader = op.reader(&p).await?;
    let mut async_reader = reader.into_futures_async_read(offset..end).await?;
    let mut bytes = Vec::with_capacity(actual_bytes as usize);
    async_reader.read_to_end(&mut bytes).await?;

    Ok(ReadFileRangeResult {
        total_size,
        offset,
        bytes,
        truncated: end < total_size,
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct PageCursor {
    version: u8,
    path: String,
    recursive: bool,
    revision: u64,
    scanned: usize,
    position: Option<String>,
}

fn encode_page_cursor(cursor: &PageCursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_page_cursor(
    encoded: &str,
    path: &str,
    recursive: bool,
    revision: u64,
) -> Result<PageCursor> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    let cursor: PageCursor = serde_json::from_slice(&bytes)
        .map_err(|_| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    if cursor.version != 2
        || cursor.path != normalize_list_path(path)
        || cursor.recursive != recursive
        || cursor.revision != revision
    {
        return Err(crate::models::CoreError::Config(
            "list cursor does not match the current query or storage revision".to_string(),
        ));
    }
    Ok(cursor)
}

fn next_page_cursor(
    path: &str,
    recursive: bool,
    revision: u64,
    scanned: usize,
    position: Option<String>,
) -> Result<String> {
    encode_page_cursor(&PageCursor {
        version: 2,
        path: normalize_list_path(path),
        recursive,
        revision,
        scanned,
        position,
    })
}

async fn page_lister(
    op: &Operator,
    path: &str,
    recursive: bool,
    start_after: Option<&str>,
) -> Result<opendal::Lister> {
    let mut builder = op.lister_with(path).recursive(recursive);
    if let Some(position) = start_after {
        builder = builder.start_after(position);
    }
    match builder.await {
        Ok(lister) => Ok(lister),
        Err(error) if path.is_empty() && error.kind() == ErrorKind::NotFound => {
            let mut fallback = op.lister_with("/").recursive(recursive);
            if let Some(position) = start_after {
                fallback = fallback.start_after(position);
            }
            Ok(fallback.await?)
        }
        Err(error) => Err(error.into()),
    }
}

async fn entry_with_metadata(op: Operator, full_path: String) -> Result<Entry> {
    let name = extract_filename(&full_path);
    let (is_dir, size, modified_at, etag) = match op.stat(&full_path).await {
        Ok(meta) => (
            meta.is_dir(),
            meta.content_length(),
            meta.last_modified().map(|dt| dt.to_string()),
            meta.etag().map(ToOwned::to_owned),
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => (false, 0, None, None),
        Err(error) => return Err(error.into()),
    };
    Ok(Entry {
        path: if is_dir {
            ensure_dir_path(&full_path)
        } else {
            full_path
        },
        name,
        is_dir,
        size: if is_dir { 0 } else { size },
        modified_at,
        etag,
    })
}

/// List entries with revision-bound opaque pagination cursors.
pub async fn list_entries_page(
    op: &Operator,
    path: &str,
    limit: u32,
    cursor: Option<String>,
    recursive: bool,
    revision: u64,
) -> Result<ListEntriesPage> {
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(crate::models::CoreError::Config(format!(
            "list limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    let cursor = match cursor.as_deref() {
        Some(cursor) => decode_page_cursor(cursor, path, recursive, revision)?,
        None => PageCursor {
            version: 2,
            path: normalize_list_path(path),
            recursive,
            revision,
            scanned: 0,
            position: None,
        },
    };
    let supports_start_after = op.info().full_capability().list_with_start_after;
    let resume_position = supports_start_after
        .then_some(cursor.position.as_deref())
        .flatten();
    let skip_count = if supports_start_after {
        0
    } else {
        cursor.scanned
    };

    if recursive {
        let p = normalize_list_path(path);
        let mut lister = page_lister(op, &p, true, resume_position).await?;
        let mut logical_index = cursor.scanned.saturating_sub(skip_count);
        let mut skipped = 0usize;
        let mut paths = Vec::with_capacity(limit as usize + 1);
        let mut capped = false;

        while let Some(object) = lister.try_next().await? {
            let full_path = object.path().to_string();
            if full_path.trim_end_matches('/').is_empty() {
                continue;
            }
            if skipped < skip_count {
                skipped += 1;
                logical_index += 1;
                continue;
            }
            if logical_index >= MAX_RECURSIVE_ITEMS as usize {
                capped = true;
                break;
            }
            if paths.len() > limit as usize {
                break;
            }
            paths.push(full_path);
            logical_index += 1;
        }
        if skipped < skip_count {
            return Err(crate::models::CoreError::Config(
                "invalid list cursor position".to_string(),
            ));
        }
        let has_more = paths.len() > limit as usize;
        paths.truncate(limit as usize);
        let entries = futures::stream::iter(
            paths
                .into_iter()
                .map(|path| entry_with_metadata(op.clone(), path)),
        )
        .buffered(16)
        .try_collect::<Vec<_>>()
        .await?;
        let next_position = entries.last().map(|entry| entry.path.clone());
        let next_scanned = cursor.scanned.saturating_add(entries.len());
        return Ok(ListEntriesPage {
            entries,
            next_cursor: has_more
                .then(|| next_page_cursor(path, true, revision, next_scanned, next_position))
                .transpose()?,
            truncated: capped,
        });
    }

    let p = normalize_list_path(path);
    let mut lister = page_lister(op, &p, false, resume_position).await?;
    let mut skipped = 0usize;
    let mut paths = Vec::with_capacity(limit as usize + 1);
    while let Some(object) = lister.try_next().await? {
        let full_path = object.path().to_string();
        if full_path.trim_end_matches('/').is_empty() {
            continue;
        }
        if skipped < skip_count {
            skipped += 1;
            continue;
        }
        if paths.len() > limit as usize {
            break;
        }
        paths.push(full_path);
    }
    if skipped < skip_count {
        return Err(crate::models::CoreError::Config(
            "invalid list cursor position".to_string(),
        ));
    }
    let has_more = paths.len() > limit as usize;
    paths.truncate(limit as usize);
    let entries = futures::stream::iter(
        paths
            .into_iter()
            .map(|path| entry_with_metadata(op.clone(), path)),
    )
    .buffered(16)
    .try_collect::<Vec<_>>()
    .await?;
    let next_position = entries.last().map(|entry| entry.path.clone());
    let next_scanned = cursor.scanned.saturating_add(entries.len());
    Ok(ListEntriesPage {
        entries,
        next_cursor: has_more
            .then(|| next_page_cursor(path, false, revision, next_scanned, next_position))
            .transpose()?,
        truncated: false,
    })
}

/// Write the full contents of a file, overwriting if it exists.
pub async fn write_full(op: &Operator, path: &str, data: &[u8]) -> Result<()> {
    write_full_with_user_metadata(op, path, data, None).await
}

/// Write the full contents of a file with optional user metadata.
///
/// Metadata is only sent when the OpenDAL backend advertises
/// `write_with_user_metadata`; callers get an explicit config error instead of
/// silently dropping requested metadata on unsupported backends.
pub async fn write_full_with_user_metadata(
    op: &Operator,
    path: &str,
    data: &[u8],
    user_metadata: Option<HashMap<String, String>>,
) -> Result<()> {
    let p = normalize_opendal_path(path);
    let Some(metadata) = sanitize_user_metadata(user_metadata) else {
        op.write(&p, data.to_vec()).await?;
        return Ok(());
    };

    if !op.info().full_capability().write_with_user_metadata {
        return Err(crate::models::CoreError::Config(
            "storage backend does not support user metadata writes".to_string(),
        ));
    }

    op.write_with(&p, data.to_vec())
        .user_metadata(metadata)
        .await?;
    Ok(())
}

fn sanitize_user_metadata(
    user_metadata: Option<HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    let metadata = user_metadata?;
    let sanitized = metadata
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_string();
            if key.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect::<HashMap<_, _>>();

    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// Create a directory at the given path.
pub async fn create_directory(op: &Operator, path: &str) -> Result<()> {
    let p = normalize_list_path(path);
    op.create_dir(&p).await?;
    Ok(())
}

/// Delete a path (file or directory).
pub async fn delete(op: &Operator, path: &str) -> Result<()> {
    let p = normalize_opendal_path(path);
    if p.is_empty() {
        return Err(crate::models::CoreError::Config(
            "refusing to delete storage root".to_string(),
        ));
    }
    delete_recursive(op, &p).await?;
    Ok(())
}

async fn delete_recursive(op: &Operator, path: &str) -> Result<()> {
    op.delete_with(path).recursive(true).await?;
    Ok(())
}

/// Upload files from local paths to the target directory.
pub async fn upload_files_from_paths(
    op: &Operator,
    paths: Vec<String>,
    target_dir: String,
) -> Result<()> {
    for path_str in paths {
        let path = Path::new(&path_str);
        upload_path_recursive(op, path, &target_dir).await?;
    }
    Ok(())
}

fn join_target_dir(base: &str, name: &str) -> String {
    if base.is_empty() || base == "/" {
        name.to_string()
    } else if base.ends_with('/') {
        format!("{}{}", base, name)
    } else {
        format!("{}/{}", base, name)
    }
}

fn ensure_dir_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        "/".to_string()
    } else if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{}/", path)
    }
}

fn parent_dir_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let idx = trimmed.rfind('/')?;
    let parent = &trimmed[..idx + 1];
    if parent.is_empty() || parent == "/" {
        None
    } else {
        Some(parent.to_string())
    }
}

fn is_current_dir_marker(base: &str, child_path: &str) -> bool {
    ensure_dir_path(base) == ensure_dir_path(child_path)
}

fn normalize_transfer_inputs(paths: Vec<String>, target_dir: &str) -> (Vec<String>, String) {
    (
        paths
            .into_iter()
            .map(|path| normalize_opendal_path(&path))
            .collect(),
        normalize_opendal_path(target_dir),
    )
}

async fn ensure_no_batch_destination_conflicts(
    from_op: &Operator,
    paths: &[String],
    target_dir: &str,
) -> Result<()> {
    let mut destinations = HashSet::new();
    for from_path in paths {
        let meta = stat_for_transfer(from_op, from_path).await?;
        let name = extract_filename(from_path);
        let destination = if meta.is_dir() {
            ensure_dir_path(&join_target_dir(target_dir, &name))
        } else {
            join_target_dir(target_dir, &name)
        };
        let key = destination.trim_end_matches('/').to_string();
        if !destinations.insert(key) {
            return Err(opendal::Error::new(
                ErrorKind::AlreadyExists,
                "Multiple selected items resolve to the same destination name",
            )
            .into());
        }
    }
    Ok(())
}

fn ensure_not_folder_into_descendant(
    from_dir: &str,
    to_dir: &str,
    same_source: bool,
) -> Result<()> {
    if !same_source {
        return Ok(());
    }

    let normalized_src = ensure_dir_path(&normalize_opendal_path(from_dir));
    let normalized_dest = ensure_dir_path(&normalize_opendal_path(to_dir));
    if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src {
        return Err(opendal::Error::new(
            ErrorKind::IsSameFile,
            "Cannot copy or move a folder into itself",
        )
        .into());
    }

    Ok(())
}

async fn ensure_parent_dir(op: &Operator, path: &str) -> Result<()> {
    if let Some(parent) = parent_dir_path(path) {
        let parent_dir = ensure_dir_path(&parent);
        op.create_dir(&parent_dir).await?;
    }
    Ok(())
}

async fn stat_for_transfer(op: &Operator, path: &str) -> Result<Metadata> {
    match op.stat(path).await {
        Ok(meta) => Ok(meta),
        Err(error)
            if error.kind() == ErrorKind::NotFound && !path.is_empty() && !path.ends_with('/') =>
        {
            op.stat(&ensure_dir_path(path)).await.map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

async fn path_exists_for_transfer(op: &Operator, path: &str) -> Result<bool> {
    match op.exists(path).await {
        Ok(exists) => Ok(exists),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => match path_exists_by_listing(op, path).await {
            Ok(exists) => Ok(exists),
            Err(_) => Err(error.into()),
        },
    }
}

async fn path_exists_by_listing(op: &Operator, path: &str) -> Result<bool> {
    let normalized = normalize_opendal_path(path);
    if normalized.is_empty() {
        return Ok(true);
    }

    let parent = parent_dir_path(&normalized).unwrap_or_default();
    let list_base = if parent.is_empty() {
        String::new()
    } else {
        ensure_dir_path(&parent)
    };
    let mut lister = match op.lister(&list_base).await {
        Ok(lister) => lister,
        Err(error) if list_base.is_empty() && error.kind() == ErrorKind::NotFound => {
            op.lister("/").await?
        }
        Err(error) => return Err(error.into()),
    };

    let expected = normalized.trim_end_matches('/');
    while let Some(obj) = lister.try_next().await? {
        if obj.path().trim_end_matches('/') == expected {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn copy_file_across_operators(
    from_op: &Operator,
    to_op: &Operator,
    from: &str,
    to: &str,
) -> Result<()> {
    copy_file_across_operators_with_progress(from_op, to_op, from, to, None::<fn(u64)>, None).await
}

async fn copy_file_across_operators_with_progress<P>(
    from_op: &Operator,
    to_op: &Operator,
    from: &str,
    to: &str,
    mut on_bytes: Option<P>,
    is_cancelled: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<()>
where
    P: FnMut(u64),
{
    let meta = stat_for_transfer(from_op, from).await?;
    let size = meta.content_length();
    let mut reader = from_op
        .reader(from)
        .await?
        .into_futures_async_read(0..size)
        .await?;
    let mut writer = match to_op.writer(to).await {
        Ok(writer) => writer.into_futures_async_write(),
        Err(error) => {
            let _ = to_op.delete(to).await;
            return Err(error.into());
        }
    };
    let mut buffer = vec![0_u8; 64 * 1024];
    let transfer = async {
        let mut transferred = 0_u64;
        loop {
            if let Some(callback) = is_cancelled {
                ensure_not_cancelled(callback)?;
            }
            let read = reader.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read]).await?;
            transferred = transferred.saturating_add(read as u64);
            if let Some(callback) = on_bytes.as_mut() {
                callback(read as u64);
            }
            if let Some(callback) = is_cancelled {
                ensure_not_cancelled(callback)?;
            }
        }
        writer.close().await?;
        if transferred != size {
            return Err(crate::models::CoreError::Config(format!(
                "cross-storage transfer byte count mismatch: expected {size}, transferred {transferred}"
            )));
        }
        let persisted = to_op.stat(to).await?.content_length();
        if persisted != size {
            return Err(crate::models::CoreError::Config(format!(
                "cross-storage transfer verification failed: expected {size}, found {persisted}"
            )));
        }
        Ok(())
    }
    .await;

    if transfer.is_err() {
        let _ = to_op.delete(to).await;
    }
    transfer
}

fn split_file_name(name: &str) -> (String, String) {
    // Keep the last extension only (simple + predictable).
    if name.starts_with('.') {
        // ".env" => treat as no-extension for our purposes
        return (name.to_string(), String::new());
    }

    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), format!(".{}", ext))
        }
        _ => (name.to_string(), String::new()),
    }
}

async fn unique_destination_path(
    op: &Operator,
    target_dir: &str,
    name: &str,
    is_dir: bool,
) -> Result<String> {
    let base_path = join_target_dir(target_dir, name);
    let mut candidate = if is_dir {
        ensure_dir_path(&base_path)
    } else {
        base_path
    };

    if !path_exists_for_transfer(op, &candidate).await? {
        return Ok(candidate);
    }

    if is_dir {
        let base_name = name.to_string();
        for idx in 1..=9999u32 {
            let suffix = if idx == 1 {
                " copy".to_string()
            } else {
                format!(" copy {}", idx)
            };
            let next_name = format!("{}{}", base_name, suffix);
            candidate = ensure_dir_path(&join_target_dir(target_dir, &next_name));
            if !path_exists_for_transfer(op, &candidate).await? {
                return Ok(candidate);
            }
        }
    } else {
        let (stem, ext) = split_file_name(name);
        for idx in 1..=9999u32 {
            let suffix = if idx == 1 {
                " copy".to_string()
            } else {
                format!(" copy {}", idx)
            };
            let next_name = format!("{}{}{}", stem, suffix, ext);
            candidate = join_target_dir(target_dir, &next_name);
            if !path_exists_for_transfer(op, &candidate).await? {
                return Ok(candidate);
            }
        }
    }

    Err(opendal::Error::new(
        ErrorKind::Unexpected,
        "Failed to generate a unique destination path",
    )
    .into())
}

fn summarize_transfer_plan(entries: &[TransferPlanEntry]) -> TransferPlanSummary {
    let mut summary = TransferPlanSummary::default();
    for entry in entries {
        match entry.action {
            TransferPlanAction::Create => summary.create += 1,
            TransferPlanAction::Overwrite => summary.overwrite += 1,
            TransferPlanAction::Skip => summary.skip += 1,
            TransferPlanAction::Rename => summary.rename += 1,
            TransferPlanAction::Noop => summary.noop += 1,
            TransferPlanAction::Conflict => summary.conflict += 1,
        }
        if !entry.is_dir {
            summary.total_items += 1;
            summary.total_bytes = summary.total_bytes.saturating_add(entry.size);
        }
    }
    summary
}

async fn plan_path_action(
    to_op: &Operator,
    source_path: &str,
    base_destination_path: &str,
    is_dir: bool,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
) -> Result<(String, TransferPlanAction)> {
    let normalized_source = if is_dir {
        ensure_dir_path(source_path)
    } else {
        source_path.to_string()
    };
    let normalized_destination = if is_dir {
        ensure_dir_path(base_destination_path)
    } else {
        base_destination_path.to_string()
    };

    if same_source {
        if operation == TransferOperation::Move && normalized_source == normalized_destination {
            return Ok((normalized_destination, TransferPlanAction::Noop));
        }
        if operation == TransferOperation::Copy && normalized_source == normalized_destination {
            let name = extract_filename(source_path);
            let parent = parent_dir_path(&normalized_destination).unwrap_or_default();
            let target_dir = parent.trim_end_matches('/');
            let destination = unique_destination_path(to_op, target_dir, &name, is_dir).await?;
            return Ok((destination, TransferPlanAction::Rename));
        }
    }

    if !path_exists_for_transfer(to_op, &normalized_destination).await? {
        return Ok((normalized_destination, TransferPlanAction::Create));
    }

    match conflict_policy {
        TransferConflictPolicy::Fail => Ok((normalized_destination, TransferPlanAction::Conflict)),
        TransferConflictPolicy::Overwrite => {
            Ok((normalized_destination, TransferPlanAction::Overwrite))
        }
        TransferConflictPolicy::Skip => Ok((normalized_destination, TransferPlanAction::Skip)),
        TransferConflictPolicy::Rename => {
            let name = extract_filename(source_path);
            let parent = parent_dir_path(&normalized_destination).unwrap_or_default();
            let target_dir = parent.trim_end_matches('/');
            let destination = unique_destination_path(to_op, target_dir, &name, is_dir).await?;
            Ok((destination, TransferPlanAction::Rename))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn collect_transfer_plan_entries(
    from_op: &Operator,
    to_op: &Operator,
    source_path: &str,
    destination_path: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
    entries: &mut Vec<TransferPlanEntry>,
) -> Result<()> {
    let meta = stat_for_transfer(from_op, source_path).await?;
    let is_dir = meta.is_dir();
    let size = if is_dir { 0 } else { meta.content_length() };
    let (planned_destination, action) = plan_path_action(
        to_op,
        source_path,
        destination_path,
        is_dir,
        operation,
        same_source,
        conflict_policy,
    )
    .await?;

    entries.push(TransferPlanEntry {
        source_path: source_path.to_string(),
        destination_path: planned_destination.clone(),
        is_dir,
        size,
        action,
    });

    if !is_dir
        || matches!(
            action,
            TransferPlanAction::Skip | TransferPlanAction::Conflict | TransferPlanAction::Noop
        )
    {
        return Ok(());
    }

    let from_root = ensure_dir_path(source_path);
    let to_root = ensure_dir_path(&planned_destination);
    let mut stack = vec![(from_root, to_root)];
    while let Some((from_base, to_base)) = stack.pop() {
        let mut lister = from_op.lister(&from_base).await?;
        while let Some(obj) = lister.try_next().await? {
            let child_path = obj.path().to_string();
            if is_current_dir_marker(&from_base, &child_path) {
                continue;
            }
            let child_meta = stat_for_transfer(from_op, &child_path).await?;
            let name = extract_filename(&child_path);
            let child_destination = if child_meta.is_dir() {
                ensure_dir_path(&join_target_dir(&to_base, &name))
            } else {
                join_target_dir(&to_base, &name)
            };
            let child_is_dir = child_meta.is_dir();
            let child_size = if child_is_dir {
                0
            } else {
                child_meta.content_length()
            };
            let (planned_child_destination, child_action) = plan_path_action(
                to_op,
                &child_path,
                &child_destination,
                child_is_dir,
                TransferOperation::Copy,
                same_source,
                conflict_policy,
            )
            .await?;
            entries.push(TransferPlanEntry {
                source_path: child_path.clone(),
                destination_path: planned_child_destination.clone(),
                is_dir: child_is_dir,
                size: child_size,
                action: child_action,
            });
            if child_is_dir
                && !matches!(
                    child_action,
                    TransferPlanAction::Skip
                        | TransferPlanAction::Conflict
                        | TransferPlanAction::Noop
                )
            {
                stack.push((ensure_dir_path(&child_path), planned_child_destination));
            }
        }
    }

    Ok(())
}

async fn estimate_transfer_totals(op: &Operator, paths: &[String]) -> Result<(u64, u64)> {
    let mut total_items = 0_u64;
    let mut total_bytes = 0_u64;

    for path in paths {
        let (items, bytes) = estimate_path_totals(op, path).await?;
        total_items = total_items.saturating_add(items);
        total_bytes = total_bytes.saturating_add(bytes);
    }

    Ok((total_items, total_bytes))
}

async fn estimate_path_totals(op: &Operator, path: &str) -> Result<(u64, u64)> {
    let meta = stat_for_transfer(op, path).await?;
    if !meta.is_dir() {
        return Ok((1, meta.content_length()));
    }

    let mut total_items = 0_u64;
    let mut total_bytes = 0_u64;
    let mut stack = vec![ensure_dir_path(path)];

    while let Some(dir) = stack.pop() {
        let mut lister = op.lister(&dir).await?;
        while let Some(obj) = lister.try_next().await? {
            let child_path = obj.path().to_string();
            if is_current_dir_marker(&dir, &child_path) {
                continue;
            }
            let child_meta = stat_for_transfer(op, &child_path).await?;
            if child_meta.is_dir() {
                stack.push(ensure_dir_path(&child_path));
            } else {
                total_items = total_items.saturating_add(1);
                total_bytes = total_bytes.saturating_add(child_meta.content_length());
            }
        }
    }

    Ok((total_items, total_bytes))
}

fn emit_progress<P>(
    progress: &mut P,
    state: &TransferProgressState,
    current_path: impl Into<String>,
) where
    P: FnMut(TransferProgress),
{
    progress(TransferProgress {
        completed_items: state.completed_items,
        total_items: state.total_items,
        bytes_transferred: state.bytes_transferred,
        total_bytes: state.total_bytes,
        current_path: current_path.into(),
    });
}

fn ensure_not_cancelled<C>(is_cancelled: &C) -> Result<()>
where
    C: Fn() -> bool + ?Sized,
{
    if is_cancelled() {
        return Err(opendal::Error::new(ErrorKind::Unexpected, "Transfer cancelled").into());
    }
    Ok(())
}

async fn transfer_file(
    from_op: &Operator,
    to_op: &Operator,
    from_path: &str,
    to_path: &str,
    operation: TransferOperation,
    same_source: bool,
) -> Result<()> {
    ensure_parent_dir(to_op, to_path).await?;

    match operation {
        TransferOperation::Copy => {
            if same_source {
                match from_op.copy(from_path, to_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::Unsupported => {
                        copy_file_across_operators(from_op, to_op, from_path, to_path).await?;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                copy_file_across_operators(from_op, to_op, from_path, to_path).await?;
            }
        }
        TransferOperation::Move => {
            if same_source {
                match from_op.rename(from_path, to_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::Unsupported => {
                        copy_file_across_operators(from_op, to_op, from_path, to_path).await?;
                        delete_recursive(from_op, from_path).await?;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                copy_file_across_operators(from_op, to_op, from_path, to_path).await?;
                delete_recursive(from_op, from_path).await?;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn transfer_file_with_progress<P, C>(
    from_op: &Operator,
    to_op: &Operator,
    from_path: &str,
    to_path: &str,
    operation: TransferOperation,
    same_source: bool,
    state: &mut TransferProgressState,
    progress: &mut P,
    is_cancelled: &C,
) -> Result<()>
where
    P: FnMut(TransferProgress),
    C: Fn() -> bool + Sync,
{
    ensure_not_cancelled(is_cancelled)?;
    ensure_parent_dir(to_op, to_path).await?;
    let size = stat_for_transfer(from_op, from_path)
        .await?
        .content_length();

    match operation {
        TransferOperation::Copy => {
            if same_source {
                match from_op.copy(from_path, to_path).await {
                    Ok(()) => {
                        state.bytes_transferred = state.bytes_transferred.saturating_add(size);
                        emit_progress(progress, state, from_path);
                    }
                    Err(error) if error.kind() == ErrorKind::Unsupported => {
                        copy_file_across_operators_with_progress(
                            from_op,
                            to_op,
                            from_path,
                            to_path,
                            Some(|bytes| {
                                state.bytes_transferred =
                                    state.bytes_transferred.saturating_add(bytes);
                                emit_progress(progress, state, from_path);
                            }),
                            Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
                        )
                        .await?;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                copy_file_across_operators_with_progress(
                    from_op,
                    to_op,
                    from_path,
                    to_path,
                    Some(|bytes| {
                        state.bytes_transferred = state.bytes_transferred.saturating_add(bytes);
                        emit_progress(progress, state, from_path);
                    }),
                    Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
                )
                .await?;
            }
        }
        TransferOperation::Move => {
            if same_source {
                match from_op.rename(from_path, to_path).await {
                    Ok(()) => {
                        state.bytes_transferred = state.bytes_transferred.saturating_add(size);
                        emit_progress(progress, state, from_path);
                    }
                    Err(error) if error.kind() == ErrorKind::Unsupported => {
                        copy_file_across_operators_with_progress(
                            from_op,
                            to_op,
                            from_path,
                            to_path,
                            Some(|bytes| {
                                state.bytes_transferred =
                                    state.bytes_transferred.saturating_add(bytes);
                                emit_progress(progress, state, from_path);
                            }),
                            Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
                        )
                        .await?;
                        ensure_not_cancelled(is_cancelled)?;
                        delete_recursive(from_op, from_path).await?;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                copy_file_across_operators_with_progress(
                    from_op,
                    to_op,
                    from_path,
                    to_path,
                    Some(|bytes| {
                        state.bytes_transferred = state.bytes_transferred.saturating_add(bytes);
                        emit_progress(progress, state, from_path);
                    }),
                    Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
                )
                .await?;
                ensure_not_cancelled(is_cancelled)?;
                delete_recursive(from_op, from_path).await?;
            }
        }
    }

    state.completed_items = state.completed_items.saturating_add(1);
    state.bytes_transferred = state.bytes_transferred.max(size.min(state.total_bytes));
    emit_progress(progress, state, from_path);
    Ok(())
}

async fn transfer_dir_recursive(
    from_op: &Operator,
    to_op: &Operator,
    from_dir: &str,
    to_dir: &str,
    operation: TransferOperation,
    same_source: bool,
) -> Result<()> {
    ensure_not_folder_into_descendant(from_dir, to_dir, same_source)?;
    let from_root = ensure_dir_path(from_dir);
    let to_root = ensure_dir_path(to_dir);
    to_op.create_dir(&to_root).await?;

    let mut stack = vec![(from_root.clone(), to_root)];
    while let Some((from_base, to_base)) = stack.pop() {
        let mut lister = from_op.lister(&from_base).await?;
        while let Some(obj) = lister.try_next().await? {
            let child_path = obj.path().to_string();
            if is_current_dir_marker(&from_base, &child_path) {
                continue;
            }
            let meta = stat_for_transfer(from_op, &child_path).await?;
            let name = extract_filename(&child_path);

            if meta.is_dir() {
                let child_src_dir = ensure_dir_path(&child_path);
                let child_dst_dir = ensure_dir_path(&join_target_dir(&to_base, &name));
                to_op.create_dir(&child_dst_dir).await?;
                stack.push((child_src_dir, child_dst_dir));
            } else {
                let child_dst_file = join_target_dir(&to_base, &name);
                transfer_file(
                    from_op,
                    to_op,
                    &child_path,
                    &child_dst_file,
                    TransferOperation::Copy,
                    same_source,
                )
                .await?;
            }
        }
    }

    if operation == TransferOperation::Move {
        delete_recursive(from_op, &from_root).await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn transfer_dir_recursive_with_progress<P, C>(
    from_op: &Operator,
    to_op: &Operator,
    from_dir: &str,
    to_dir: &str,
    operation: TransferOperation,
    same_source: bool,
    state: &mut TransferProgressState,
    progress: &mut P,
    is_cancelled: &C,
) -> Result<()>
where
    P: FnMut(TransferProgress),
    C: Fn() -> bool + Sync,
{
    ensure_not_cancelled(is_cancelled)?;
    ensure_not_folder_into_descendant(from_dir, to_dir, same_source)?;
    let from_root = ensure_dir_path(from_dir);
    let to_root = ensure_dir_path(to_dir);
    to_op.create_dir(&to_root).await?;

    let mut stack = vec![(from_root.clone(), to_root)];
    while let Some((from_base, to_base)) = stack.pop() {
        ensure_not_cancelled(is_cancelled)?;
        let mut lister = from_op.lister(&from_base).await?;
        while let Some(obj) = lister.try_next().await? {
            ensure_not_cancelled(is_cancelled)?;
            let child_path = obj.path().to_string();
            if is_current_dir_marker(&from_base, &child_path) {
                continue;
            }
            let meta = stat_for_transfer(from_op, &child_path).await?;
            let name = extract_filename(&child_path);

            if meta.is_dir() {
                let child_src_dir = ensure_dir_path(&child_path);
                let child_dst_dir = ensure_dir_path(&join_target_dir(&to_base, &name));
                to_op.create_dir(&child_dst_dir).await?;
                stack.push((child_src_dir, child_dst_dir));
            } else {
                let child_dst_file = join_target_dir(&to_base, &name);
                transfer_file_with_progress(
                    from_op,
                    to_op,
                    &child_path,
                    &child_dst_file,
                    TransferOperation::Copy,
                    same_source,
                    state,
                    progress,
                    is_cancelled,
                )
                .await?;
            }
        }
    }

    if operation == TransferOperation::Move {
        ensure_not_cancelled(is_cancelled)?;
        delete_recursive(from_op, &from_root).await?;
    }

    Ok(())
}

/// Build a backend-agnostic dry-run manifest for a copy or move.
///
/// The plan performs only OpenDAL stat/list/exists calls. It does not mutate storage.
pub async fn plan_transfer_entries(
    from_op: &Operator,
    to_op: &Operator,
    paths: Vec<String>,
    target_dir: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
) -> Result<TransferPlan> {
    let (paths, target_dir) = normalize_transfer_inputs(paths, target_dir);
    let target_dir = target_dir.as_str();
    let mut entries = Vec::new();

    for source_path in &paths {
        let meta = stat_for_transfer(from_op, source_path).await?;
        let name = extract_filename(source_path);
        let destination = if meta.is_dir() {
            ensure_dir_path(&join_target_dir(target_dir, &name))
        } else {
            join_target_dir(target_dir, &name)
        };
        collect_transfer_plan_entries(
            from_op,
            to_op,
            source_path,
            &destination,
            operation,
            same_source,
            conflict_policy,
            &mut entries,
        )
        .await?;
    }

    let summary = summarize_transfer_plan(&entries);
    Ok(TransferPlan {
        operation,
        conflict_policy,
        entries,
        summary,
    })
}

/// Transfer a single path (file or directory) to a specific destination path.
///
/// This is a lower-level primitive than `transfer_entries` because it allows
/// renaming the top-level item during the transfer.
pub async fn transfer_path(
    from_op: &Operator,
    to_op: &Operator,
    source_path: &str,
    destination_path: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
) -> Result<()> {
    let source_path = normalize_opendal_path(source_path);
    let destination_path = normalize_opendal_path(destination_path);

    let meta = stat_for_transfer(from_op, &source_path).await?;
    if meta.is_dir() {
        let normalized_src = ensure_dir_path(&source_path);
        let normalized_dest = ensure_dir_path(&destination_path);

        if same_source {
            if operation == TransferOperation::Move && normalized_src == normalized_dest {
                return Ok(());
            }
            if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src {
                return Err(opendal::Error::new(
                    ErrorKind::IsSameFile,
                    "Cannot copy a folder into itself",
                )
                .into());
            }
        }

        if path_exists_for_transfer(to_op, &normalized_dest).await? {
            match conflict_policy {
                TransferConflictPolicy::Fail => {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination directory already exists",
                    )
                    .into())
                }
                TransferConflictPolicy::Overwrite => {
                    delete_recursive(to_op, &normalized_dest).await?;
                }
                TransferConflictPolicy::Skip => {
                    return Ok(());
                }
                TransferConflictPolicy::Rename => {
                    // Rename logic for a specific destination path is complex if it already exists.
                    // Usually transfer_entries handles this by generating a unique name in target_dir.
                    // For transfer_path, we expect the caller to have resolved the destination.
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination directory already exists and Rename policy not implemented for transfer_path",
                    )
                    .into());
                }
            }
        }

        transfer_dir_recursive(
            from_op,
            to_op,
            &normalized_src,
            &normalized_dest,
            operation,
            same_source,
        )
        .await?;
    } else {
        if operation == TransferOperation::Move && same_source && source_path == destination_path {
            return Ok(());
        }

        if path_exists_for_transfer(to_op, &destination_path).await? {
            match conflict_policy {
                TransferConflictPolicy::Fail => {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination file already exists",
                    )
                    .into())
                }
                TransferConflictPolicy::Overwrite => {
                    delete_recursive(to_op, &destination_path).await?;
                }
                TransferConflictPolicy::Skip => {
                    return Ok(());
                }
                TransferConflictPolicy::Rename => {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination file already exists and Rename policy not implemented for transfer_path",
                    )
                    .into());
                }
            }
        }

        transfer_file(
            from_op,
            to_op,
            &source_path,
            &destination_path,
            operation,
            same_source,
        )
        .await?;
    }

    Ok(())
}

/// Copy or move a set of file/folder paths into `target_dir`.
///
/// Conflict handling is controlled by `conflict_policy`.
///
/// Note: copying an entry onto itself (same source + same path) is treated as a duplicate copy
/// and the destination name is auto-deduplicated to avoid clobbering the source.
pub async fn transfer_entries(
    from_op: &Operator,
    to_op: &Operator,
    paths: Vec<String>,
    target_dir: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
) -> Result<()> {
    let (paths, target_dir) = normalize_transfer_inputs(paths, target_dir);
    let target_dir = target_dir.as_str();

    if conflict_policy == TransferConflictPolicy::Fail {
        ensure_no_batch_destination_conflicts(from_op, &paths, target_dir).await?;
        for from_path in &paths {
            let meta = stat_for_transfer(from_op, from_path).await?;

            if meta.is_dir() {
                let dir_name = extract_filename(from_path);
                let dest_dir = ensure_dir_path(&join_target_dir(target_dir, &dir_name));
                let normalized_src = ensure_dir_path(from_path);
                let normalized_dest = ensure_dir_path(&dest_dir);

                if same_source {
                    if operation == TransferOperation::Move && normalized_src == normalized_dest {
                        continue;
                    }
                    if normalized_dest.starts_with(&normalized_src)
                        && normalized_dest != normalized_src
                    {
                        return Err(opendal::Error::new(
                            ErrorKind::IsSameFile,
                            "Cannot copy a folder into itself",
                        )
                        .into());
                    }
                }

                // Copying onto itself is treated as "duplicate" (keep both) and won't conflict.
                if operation == TransferOperation::Copy
                    && same_source
                    && normalized_src == normalized_dest
                {
                    continue;
                }

                if path_exists_for_transfer(to_op, &dest_dir).await? {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination directory already exists",
                    )
                    .into());
                }
            } else {
                let file_name = extract_filename(from_path);
                let dest_file = join_target_dir(target_dir, &file_name);

                if same_source {
                    if operation == TransferOperation::Move && *from_path == dest_file {
                        continue;
                    }
                    if operation == TransferOperation::Copy && *from_path == dest_file {
                        continue;
                    }
                }

                if path_exists_for_transfer(to_op, &dest_file).await? {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination file already exists",
                    )
                    .into());
                }
            }
        }
    }

    for from_path in paths {
        let meta = stat_for_transfer(from_op, &from_path).await?;
        if meta.is_dir() {
            let dir_name = extract_filename(&from_path);
            let base_dest_dir = ensure_dir_path(&join_target_dir(target_dir, &dir_name));
            let normalized_src = ensure_dir_path(&from_path);
            let normalized_dest = ensure_dir_path(&base_dest_dir);

            if same_source {
                if operation == TransferOperation::Move && normalized_src == normalized_dest {
                    continue;
                }
                if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src
                {
                    return Err(opendal::Error::new(
                        ErrorKind::IsSameFile,
                        "Cannot copy a folder into itself",
                    )
                    .into());
                }
            }

            let dest_dir = if operation == TransferOperation::Copy
                && same_source
                && normalized_src == normalized_dest
            {
                unique_destination_path(to_op, target_dir, &dir_name, true).await?
            } else {
                base_dest_dir
            };

            if path_exists_for_transfer(to_op, &dest_dir).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination directory already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        delete_recursive(to_op, &dest_dir).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                    TransferConflictPolicy::Rename => {
                        // The non-conflicting path is selected before this branch.
                    }
                }
            }

            let dest_dir = if conflict_policy == TransferConflictPolicy::Rename
                && path_exists_for_transfer(to_op, &dest_dir).await?
            {
                unique_destination_path(to_op, target_dir, &dir_name, true).await?
            } else {
                dest_dir
            };

            transfer_dir_recursive(
                from_op,
                to_op,
                &ensure_dir_path(&from_path),
                &dest_dir,
                operation,
                same_source,
            )
            .await?;
        } else {
            let file_name = extract_filename(&from_path);
            let base_dest_file = join_target_dir(target_dir, &file_name);
            let dest_file = if operation == TransferOperation::Copy
                && same_source
                && from_path == base_dest_file
            {
                unique_destination_path(to_op, target_dir, &file_name, false).await?
            } else {
                base_dest_file
            };

            if operation == TransferOperation::Move && same_source && from_path == dest_file {
                continue;
            }

            if path_exists_for_transfer(to_op, &dest_file).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination file already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        delete_recursive(to_op, &dest_file).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                    TransferConflictPolicy::Rename => {
                        // The non-conflicting path is selected before this branch.
                    }
                }
            }
            let dest_file = if conflict_policy == TransferConflictPolicy::Rename
                && path_exists_for_transfer(to_op, &dest_file).await?
            {
                unique_destination_path(to_op, target_dir, &file_name, false).await?
            } else {
                dest_file
            };
            transfer_file(
                from_op,
                to_op,
                &from_path,
                &dest_file,
                operation,
                same_source,
            )
            .await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn transfer_entries_with_progress<P, C>(
    from_op: &Operator,
    to_op: &Operator,
    paths: Vec<String>,
    target_dir: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
    mut progress: P,
    is_cancelled: C,
) -> Result<()>
where
    P: FnMut(TransferProgress),
    C: Fn() -> bool + Sync,
{
    let (paths, target_dir) = normalize_transfer_inputs(paths, target_dir);
    let target_dir = target_dir.as_str();
    let (total_items, total_bytes) = estimate_transfer_totals(from_op, &paths).await?;
    let mut state = TransferProgressState {
        completed_items: 0,
        total_items,
        bytes_transferred: 0,
        total_bytes,
    };
    emit_progress(&mut progress, &state, "");

    if conflict_policy == TransferConflictPolicy::Fail {
        ensure_no_batch_destination_conflicts(from_op, &paths, target_dir).await?;
        for from_path in &paths {
            ensure_not_cancelled(&is_cancelled)?;
            let meta = stat_for_transfer(from_op, from_path).await?;

            if meta.is_dir() {
                let dir_name = extract_filename(from_path);
                let dest_dir = ensure_dir_path(&join_target_dir(target_dir, &dir_name));
                let normalized_src = ensure_dir_path(from_path);
                let normalized_dest = ensure_dir_path(&dest_dir);

                if same_source {
                    if operation == TransferOperation::Move && normalized_src == normalized_dest {
                        continue;
                    }
                    if normalized_dest.starts_with(&normalized_src)
                        && normalized_dest != normalized_src
                    {
                        return Err(opendal::Error::new(
                            ErrorKind::IsSameFile,
                            "Cannot copy a folder into itself",
                        )
                        .into());
                    }
                }

                if operation == TransferOperation::Copy
                    && same_source
                    && normalized_src == normalized_dest
                {
                    continue;
                }

                if path_exists_for_transfer(to_op, &dest_dir).await? {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination directory already exists",
                    )
                    .into());
                }
            } else {
                let file_name = extract_filename(from_path);
                let dest_file = join_target_dir(target_dir, &file_name);

                if same_source {
                    if operation == TransferOperation::Move && *from_path == dest_file {
                        continue;
                    }
                    if operation == TransferOperation::Copy && *from_path == dest_file {
                        continue;
                    }
                }

                if path_exists_for_transfer(to_op, &dest_file).await? {
                    return Err(opendal::Error::new(
                        ErrorKind::AlreadyExists,
                        "Destination file already exists",
                    )
                    .into());
                }
            }
        }
    }

    for from_path in paths {
        ensure_not_cancelled(&is_cancelled)?;
        let meta = stat_for_transfer(from_op, &from_path).await?;
        if meta.is_dir() {
            let dir_name = extract_filename(&from_path);
            let base_dest_dir = ensure_dir_path(&join_target_dir(target_dir, &dir_name));
            let normalized_src = ensure_dir_path(&from_path);
            let normalized_dest = ensure_dir_path(&base_dest_dir);

            if same_source {
                if operation == TransferOperation::Move && normalized_src == normalized_dest {
                    continue;
                }
                if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src
                {
                    return Err(opendal::Error::new(
                        ErrorKind::IsSameFile,
                        "Cannot copy a folder into itself",
                    )
                    .into());
                }
            }

            let dest_dir = if operation == TransferOperation::Copy
                && same_source
                && normalized_src == normalized_dest
            {
                unique_destination_path(to_op, target_dir, &dir_name, true).await?
            } else {
                base_dest_dir
            };

            if path_exists_for_transfer(to_op, &dest_dir).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination directory already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        ensure_not_cancelled(&is_cancelled)?;
                        delete_recursive(to_op, &dest_dir).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                    TransferConflictPolicy::Rename => {
                        // The non-conflicting path is selected before this branch.
                    }
                }
            }

            let dest_dir = if conflict_policy == TransferConflictPolicy::Rename
                && path_exists_for_transfer(to_op, &dest_dir).await?
            {
                unique_destination_path(to_op, target_dir, &dir_name, true).await?
            } else {
                dest_dir
            };

            transfer_dir_recursive_with_progress(
                from_op,
                to_op,
                &ensure_dir_path(&from_path),
                &dest_dir,
                operation,
                same_source,
                &mut state,
                &mut progress,
                &is_cancelled,
            )
            .await?;
        } else {
            let file_name = extract_filename(&from_path);
            let base_dest_file = join_target_dir(target_dir, &file_name);
            let dest_file = if operation == TransferOperation::Copy
                && same_source
                && from_path == base_dest_file
            {
                unique_destination_path(to_op, target_dir, &file_name, false).await?
            } else {
                base_dest_file
            };

            if operation == TransferOperation::Move && same_source && from_path == dest_file {
                continue;
            }

            if path_exists_for_transfer(to_op, &dest_file).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination file already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        ensure_not_cancelled(&is_cancelled)?;
                        delete_recursive(to_op, &dest_file).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                    TransferConflictPolicy::Rename => {
                        // The non-conflicting path is selected before this branch.
                    }
                }
            }
            let dest_file = if conflict_policy == TransferConflictPolicy::Rename
                && path_exists_for_transfer(to_op, &dest_file).await?
            {
                unique_destination_path(to_op, target_dir, &file_name, false).await?
            } else {
                dest_file
            };
            transfer_file_with_progress(
                from_op,
                to_op,
                &from_path,
                &dest_file,
                operation,
                same_source,
                &mut state,
                &mut progress,
                &is_cancelled,
            )
            .await?;
        }
    }

    emit_progress(&mut progress, &state, "");
    Ok(())
}

/// Stream one storage object to an exact local path without buffering the full file.
pub async fn download_file_to_local_path(
    op: &Operator,
    source_path: &str,
    local_path: &Path,
) -> Result<u64> {
    let metadata = op.stat(source_path).await?;
    if metadata.is_dir() {
        return Err(crate::models::CoreError::Config(
            "cannot download a directory as a file".to_string(),
        ));
    }
    let expected_bytes = metadata.content_length();
    let mut reader = op
        .reader(source_path)
        .await?
        .into_futures_async_read(0..expected_bytes)
        .await?;
    let mut destination = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(local_path)
        .await?;
    let result = async {
        let mut transferred = 0_u64;
        let mut buffer = vec![0_u8; 256 * 1024];
        loop {
            let read = futures::AsyncReadExt::read(&mut reader, &mut buffer).await?;
            if read == 0 {
                break;
            }
            tokio::io::AsyncWriteExt::write_all(&mut destination, &buffer[..read]).await?;
            transferred = transferred.saturating_add(read as u64);
        }
        tokio::io::AsyncWriteExt::flush(&mut destination).await?;
        destination.sync_all().await?;
        if transferred != expected_bytes {
            return Err(crate::models::CoreError::Config(format!(
                "download byte count mismatch: expected {expected_bytes}, transferred {transferred}"
            )));
        }
        let persisted = fs::metadata(local_path).await?.len();
        if persisted != expected_bytes {
            return Err(crate::models::CoreError::Config(format!(
                "download verification failed: expected {expected_bytes}, found {persisted}"
            )));
        }
        Ok(transferred)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(local_path).await;
    }
    result
}

/// Stream one local staging file to an exact storage path without buffering the full file.
pub async fn upload_local_file_to_path(
    op: &Operator,
    source_path: &Path,
    target_path: &str,
) -> Result<()> {
    stream_local_file(op, source_path, target_path).await
}

async fn stream_local_file(op: &Operator, source_path: &Path, target_path: &str) -> Result<()> {
    let expected_bytes = fs::metadata(source_path).await?.len();
    let mut source = fs::File::open(source_path).await?;
    let mut destination = op.writer(target_path).await?.into_futures_async_write();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut transferred_bytes = 0_u64;
    loop {
        let read = tokio::io::AsyncReadExt::read(&mut source, &mut buffer).await?;
        if read == 0 {
            break;
        }
        destination.write_all(&buffer[..read]).await?;
        transferred_bytes = transferred_bytes.saturating_add(read as u64);
    }
    destination.close().await?;
    if transferred_bytes != expected_bytes {
        return Err(crate::models::CoreError::Config(format!(
            "upload byte count mismatch: expected {expected_bytes}, transferred {transferred_bytes}"
        )));
    }
    let persisted_bytes = op.stat(target_path).await?.content_length();
    if persisted_bytes != expected_bytes {
        return Err(crate::models::CoreError::Config(format!(
            "upload verification failed: expected {expected_bytes}, found {persisted_bytes}"
        )));
    }
    Ok(())
}

async fn upload_path_recursive(op: &Operator, src: &Path, target_dir: &str) -> Result<()> {
    let meta = fs::metadata(src).await.map_err(|e| {
        opendal::Error::new(
            ErrorKind::Unexpected,
            format!("Failed to stat local path {}: {}", src.display(), e),
        )
    })?;

    if meta.is_file() {
        let filename = src
            .file_name()
            .ok_or_else(|| {
                opendal::Error::new(ErrorKind::Unexpected, "Invalid file path (no filename)")
            })?
            .to_string_lossy();

        let target_path = join_target_dir(target_dir, &filename);

        stream_local_file(op, src, &target_path).await?;
    } else if meta.is_dir() {
        let mut stack: Vec<(std::path::PathBuf, String)> =
            vec![(src.to_path_buf(), target_dir.to_string())];

        while let Some((dir_path, dir_target)) = stack.pop() {
            let mut entries = fs::read_dir(&dir_path).await.map_err(|e| {
                opendal::Error::new(
                    ErrorKind::Unexpected,
                    format!("Failed to read directory {}: {}", dir_path.display(), e),
                )
            })?;

            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                opendal::Error::new(
                    ErrorKind::Unexpected,
                    format!("Failed to iterate directory {}: {}", dir_path.display(), e),
                )
            })? {
                let child_path = entry.path();
                let child_meta = fs::metadata(&child_path).await.map_err(|e| {
                    opendal::Error::new(
                        ErrorKind::Unexpected,
                        format!("Failed to stat local path {}: {}", child_path.display(), e),
                    )
                })?;

                if child_meta.is_file() {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    let target_path = join_target_dir(&dir_target, &filename);
                    stream_local_file(op, &child_path, &target_path).await?;
                } else if child_meta.is_dir() {
                    let dirname = entry.file_name().to_string_lossy().to_string();
                    let new_target = join_target_dir(&dir_target, &dirname);
                    stack.push((child_path, new_target));
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileVersion {
    pub version: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListVersionsResult {
    pub path: String,
    pub versions: Vec<FileVersion>,
    pub next_cursor: Option<String>,
}

pub async fn list_file_versions(
    op: &Operator,
    path: &str,
    limit: u32,
    _cursor: Option<&str>,
) -> Result<ListVersionsResult> {
    let normalized = normalize_opendal_path(path);
    let mut versions = Vec::new();

    let mut lister = match op.lister_with(&normalized).versions(true).await {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok(ListVersionsResult {
                path: path.to_string(),
                versions: vec![],
                next_cursor: None,
            });
        }
        Err(e) => return Err(e.into()),
    };

    while let Some(entry) = lister.try_next().await? {
        let meta = entry.metadata();
        if let Some(version) = meta.version() {
            let modified_at = meta.last_modified().map(|dt| dt.to_string());
            let etag = meta.etag().map(|s| s.to_string());
            versions.push(FileVersion {
                version: version.to_string(),
                size_bytes: Some(meta.content_length()),
                modified_at,
                etag,
            });
        }
    }

    versions.sort_by(|a, b| {
        let a_time = a.modified_at.as_deref().unwrap_or("");
        let b_time = b.modified_at.as_deref().unwrap_or("");
        b_time.cmp(a_time).then_with(|| a.version.cmp(&b.version))
    });

    if limit > 0 && versions.len() > limit as usize {
        versions.truncate(limit as usize);
    }

    Ok(ListVersionsResult {
        path: path.to_string(),
        versions,
        next_cursor: None,
    })
}

pub async fn read_file_version(op: &Operator, path: &str, version: &str) -> Result<Vec<u8>> {
    let normalized = normalize_opendal_path(path);
    let data = op.read_with(&normalized).version(version).await?;
    Ok(data.to_vec())
}

pub async fn delete_file_version(op: &Operator, path: &str, version: &str) -> Result<()> {
    let normalized = normalize_opendal_path(path);
    op.delete_with(&normalized).version(version).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use opendal::services::Fs;
    use opendal::services::Memory;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    async fn create_test_operator() -> Operator {
        let builder = Memory::default();
        Operator::new(builder).unwrap().finish()
    }

    #[cfg(unix)]
    fn unique_temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "infimount-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn test_list_entries() {
        let op = create_test_operator().await;
        op.write("file1.txt", "content1".as_bytes()).await.unwrap();
        op.write("dir1/file2.txt", "content2".as_bytes())
            .await
            .unwrap();

        let entries = list_entries(&op, "/").await.unwrap();
        assert_eq!(entries.len(), 2); // file1.txt and dir1

        let file1 = entries.iter().find(|e| e.name == "file1.txt").unwrap();
        assert!(!file1.is_dir);
        assert_eq!(file1.size, 8);

        let dir1 = entries.iter().find(|e| e.name == "dir1").unwrap();
        assert!(dir1.is_dir);
    }

    #[tokio::test]
    async fn test_list_entries_recursive() {
        let op = create_test_operator().await;
        op.write("file1.txt", "content1".as_bytes()).await.unwrap();
        op.write("dir1/file2.txt", "content2".as_bytes())
            .await
            .unwrap();
        op.write("dir1/nested/file3.txt", "content3".as_bytes())
            .await
            .unwrap();

        let entries = list_entries_recursive(&op, "/").await.unwrap();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();

        assert!(paths.contains(&"file1.txt"));
        assert!(paths.contains(&"dir1/"));
        assert!(paths.contains(&"dir1/file2.txt"));
        assert!(paths.contains(&"dir1/nested/"));
        assert!(paths.contains(&"dir1/nested/file3.txt"));
    }

    #[tokio::test]
    async fn paginated_cursor_is_opaque_and_query_revision_bound() {
        let op = create_test_operator().await;
        for name in ["a.txt", "b.txt", "c.txt"] {
            op.write(name, name.as_bytes()).await.unwrap();
        }
        let first = list_entries_page(&op, "/", 2, None, false, 4)
            .await
            .unwrap();
        let cursor = first.next_cursor.unwrap();
        assert!(cursor.parse::<usize>().is_err());
        let second = list_entries_page(&op, "/", 2, Some(cursor.clone()), false, 4)
            .await
            .unwrap();
        assert_eq!(second.entries.len(), 1);
        assert!(
            list_entries_page(&op, "/other", 2, Some(cursor.clone()), false, 4)
                .await
                .is_err()
        );
        assert!(list_entries_page(&op, "/", 2, Some(cursor), false, 5)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn recursive_pages_continue_into_skipped_directories() {
        let op = create_test_operator().await;
        op.write("dir/a.txt", b"a".as_slice()).await.unwrap();
        op.write("dir/nested/b.txt", b"b".as_slice()).await.unwrap();
        let mut cursor = None;
        let mut paths = Vec::new();
        loop {
            let page = list_entries_page(&op, "/", 1, cursor, true, 1)
                .await
                .unwrap();
            paths.extend(page.entries.into_iter().map(|entry| entry.path));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert!(paths.contains(&"dir/nested/b.txt".to_string()));
    }

    #[tokio::test]
    async fn range_read_rejects_oversize_and_out_of_bounds_requests() {
        let op = create_test_operator().await;
        op.write("file", b"hello".as_slice()).await.unwrap();
        assert!(read_file_range(&op, "file", 6, 1).await.is_err());
        assert!(
            read_file_range(&op, "file", 0, crate::models::MAX_READ_RANGE_BYTES + 1)
                .await
                .is_err()
        );
        let result = read_file_range(&op, "file", 0, 0).await.unwrap();
        assert_eq!(result.bytes, b"hello");
    }

    #[tokio::test]
    #[ignore = "performance smoke benchmark; run via scripts/benchmark-pr09.sh"]
    async fn benchmark_paginated_listing() {
        let op = create_test_operator().await;
        for index in 0..10_000 {
            op.write(&format!("entry-{index:05}.txt"), b"x".as_slice())
                .await
                .unwrap();
        }
        let started = std::time::Instant::now();
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = list_entries_page(&op, "/", 500, cursor, false, 1)
                .await
                .unwrap();
            count += page.entries.len();
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(count, 10_000);
        eprintln!(
            "metric=listing_10k entries={count} elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }

    #[tokio::test]
    #[ignore = "performance smoke benchmark; run via scripts/benchmark-pr09.sh"]
    async fn benchmark_recursive_listing_100k() {
        let op = create_test_operator().await;
        futures::stream::iter(0..100_000_u32)
            .map(|index| {
                let op = op.clone();
                async move {
                    op.write(
                        &format!("group-{index:03}/entry-{index:06}.txt"),
                        b"x".as_slice(),
                    )
                    .await
                }
            })
            .buffer_unordered(256)
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let first = list_entries_page(&op, "/", 500, None, true, 1)
            .await
            .unwrap();
        let first_page_ms = started.elapsed().as_millis();
        let mut cursor = first.next_cursor;
        let mut count = first.entries.len();
        let mut truncated = first.truncated;
        while let Some(next) = cursor {
            let page = list_entries_page(&op, "/", 500, Some(next), true, 1)
                .await
                .unwrap();
            count += page.entries.len();
            truncated |= page.truncated;
            cursor = page.next_cursor;
        }
        assert_eq!(count, MAX_RECURSIVE_ITEMS as usize);
        assert!(truncated);
        eprintln!(
            "metric=recursive_100k corpus_entries=100000 capped_entries={count} first_page_ms={first_page_ms} elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "writes a 1 GiB local streaming round trip; opt in via PR09_FULL=1"]
    async fn benchmark_streaming_one_gib_local_round_trip() {
        let root = unique_temp_dir("streaming-1gib");
        let source = root.join("source.bin");
        let mut file = tokio::fs::File::create(&source).await.unwrap();
        let chunk = vec![0x5a_u8; 8 * 1024 * 1024];
        for _ in 0..128 {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .unwrap();
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await.unwrap();
        drop(file);
        let storage_root = root.join("storage");
        std::fs::create_dir_all(&storage_root).unwrap();
        let op = Operator::new(Fs::default().root(storage_root.to_str().unwrap()))
            .unwrap()
            .finish();
        let started = std::time::Instant::now();
        upload_local_file_to_path(&op, &source, "uploaded.bin")
            .await
            .unwrap();
        let downloaded = root.join("downloaded.bin");
        let bytes = download_file_to_local_path(&op, "uploaded.bin", &downloaded)
            .await
            .unwrap();
        assert_eq!(bytes, 1024 * 1024 * 1024);
        eprintln!(
            "metric=local_round_trip_1gib bytes={bytes} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn test_read_write_file() {
        let op = create_test_operator().await;
        let path = "test.txt";
        let content = b"hello world";

        write_full(&op, path, content).await.unwrap();

        let read_content = read_full(&op, path).await.unwrap();
        assert_eq!(read_content, content);
    }

    #[tokio::test]
    async fn write_full_with_user_metadata_rejects_unsupported_backends() {
        let op = create_test_operator().await;
        assert!(!op.info().full_capability().write_with_user_metadata);

        let err = write_full_with_user_metadata(
            &op,
            "metadata.txt",
            b"hello",
            Some(HashMap::from([(
                "language".to_string(),
                "rust".to_string(),
            )])),
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("storage backend does not support user metadata writes"));
    }

    #[tokio::test]
    async fn write_full_with_empty_user_metadata_falls_back_to_plain_write() {
        let op = create_test_operator().await;

        write_full_with_user_metadata(
            &op,
            "metadata.txt",
            b"hello",
            Some(HashMap::from([(" ".to_string(), "ignored".to_string())])),
        )
        .await
        .unwrap();

        assert_eq!(op.read("metadata.txt").await.unwrap().to_vec(), b"hello");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let op = create_test_operator().await;
        let path = "todelete.txt";
        op.write(path, "bye".as_bytes()).await.unwrap();

        delete(&op, path).await.unwrap();

        let exists = op.exists(path).await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_delete_refuses_storage_root() {
        let op = create_test_operator().await;
        op.write("keep.txt", "safe".as_bytes()).await.unwrap();

        let slash_error = delete(&op, "/").await.unwrap_err().to_string();
        let empty_error = delete(&op, "").await.unwrap_err().to_string();

        assert!(slash_error.contains("refusing to delete storage root"));
        assert!(empty_error.contains("refusing to delete storage root"));
        assert_eq!(op.read("keep.txt").await.unwrap().to_vec(), b"safe");
    }

    #[tokio::test]
    async fn test_create_directory() {
        let op = create_test_operator().await;
        create_directory(&op, "new-folder").await.unwrap();
        let exists = op.exists("new-folder/").await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_transfer_entries_copies_file_across_operators() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op
            .write("source.txt", "hello".as_bytes())
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();

        transfer_entries(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        assert_eq!(from_op.read("source.txt").await.unwrap().to_vec(), b"hello");
        assert_eq!(
            to_op.read("target/source.txt").await.unwrap().to_vec(),
            b"hello"
        );
    }

    #[tokio::test]
    async fn test_transfer_entries_moves_file_across_operators() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op
            .write("source.txt", "hello".as_bytes())
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();

        transfer_entries(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Move,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        assert!(!from_op.exists("source.txt").await.unwrap());
        assert_eq!(
            to_op.read("target/source.txt").await.unwrap().to_vec(),
            b"hello"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_transfer_entries_move_preserves_source_when_destination_write_fails() {
        let from_op = create_test_operator().await;
        from_op
            .write("source.txt", "hello".as_bytes())
            .await
            .unwrap();

        let root = unique_temp_dir("move-write-fails");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o500)).unwrap();

        let to_op = Operator::new(Fs::default().root(root.to_str().unwrap()))
            .unwrap()
            .finish();

        let result = transfer_entries(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Move,
            false,
            TransferConflictPolicy::Fail,
        )
        .await;

        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert!(result.is_err());
        assert_eq!(from_op.read("source.txt").await.unwrap().to_vec(), b"hello");
    }

    #[tokio::test]
    async fn test_transfer_entries_with_progress_reports_totals() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op
            .write("source.txt", "hello".as_bytes())
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();

        let mut events = Vec::new();
        transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            |progress| events.push(progress),
            || false,
        )
        .await
        .unwrap();

        let final_event = events.last().expect("progress events should be emitted");
        assert_eq!(final_event.total_items, 1);
        assert_eq!(final_event.completed_items, 1);
        assert_eq!(final_event.total_bytes, 5);
        assert_eq!(final_event.bytes_transferred, 5);
    }

    #[tokio::test]
    async fn download_to_local_path_streams_and_verifies_bytes() {
        let op = create_test_operator().await;
        let bytes = vec![42_u8; 2 * 1024 * 1024];
        op.write("large.bin", bytes.clone()).await.unwrap();
        let directory = unique_temp_dir("stream-download");
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("large.bin");

        let transferred = download_file_to_local_path(&op, "large.bin", &destination)
            .await
            .unwrap();
        assert_eq!(transferred, bytes.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn test_transfer_entries_with_progress_honors_cancellation() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op
            .write("source.txt", "hello".as_bytes())
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();

        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            |_| {},
            || true,
        )
        .await;

        assert!(result.is_err());
        assert!(!to_op.exists("target/source.txt").await.unwrap());
    }

    #[tokio::test]
    async fn mid_stream_cancellation_removes_partial_cross_storage_destination() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op
            .write("source.bin", vec![7_u8; 1024 * 1024])
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let set_cancelled = Arc::clone(&cancelled);

        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["source.bin".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.bytes_transferred > 0 {
                    set_cancelled.store(true, Ordering::SeqCst);
                }
            },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;

        assert!(result.is_err());
        assert!(!to_op.exists("target/source.bin").await.unwrap());
    }

    #[tokio::test]
    async fn test_plan_transfer_entries_reports_conflicts_without_mutation() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op.write("source.txt", "new".as_bytes()).await.unwrap();
        create_directory(&to_op, "target").await.unwrap();
        to_op
            .write("target/source.txt", "old".as_bytes())
            .await
            .unwrap();

        let plan = plan_transfer_entries(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        assert_eq!(plan.summary.conflict, 1);
        assert_eq!(plan.summary.total_items, 1);
        assert_eq!(plan.summary.total_bytes, 3);
        assert_eq!(plan.entries[0].action, TransferPlanAction::Conflict);
        assert_eq!(plan.entries[0].destination_path, "target/source.txt");
        assert_eq!(
            to_op.read("target/source.txt").await.unwrap().to_vec(),
            b"old"
        );
    }

    #[tokio::test]
    async fn test_transfer_entries_renames_existing_destination() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op.write("source.txt", "new".as_bytes()).await.unwrap();
        create_directory(&to_op, "target").await.unwrap();
        to_op
            .write("target/source.txt", "old".as_bytes())
            .await
            .unwrap();

        transfer_entries(
            &from_op,
            &to_op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Rename,
        )
        .await
        .unwrap();

        assert_eq!(from_op.read("source.txt").await.unwrap().to_vec(), b"new");
        assert_eq!(
            to_op.read("target/source.txt").await.unwrap().to_vec(),
            b"old"
        );
        assert_eq!(
            to_op.read("target/source copy.txt").await.unwrap().to_vec(),
            b"new"
        );
    }

    #[tokio::test]
    async fn test_transfer_entries_copies_directory_recursively() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op.create_dir("docs/").await.unwrap();
        from_op.create_dir("docs/empty/").await.unwrap();
        from_op.write("docs/a.txt", "a".as_bytes()).await.unwrap();
        from_op
            .write("docs/nested/b.txt", "b".as_bytes())
            .await
            .unwrap();
        create_directory(&to_op, "target").await.unwrap();

        transfer_entries(
            &from_op,
            &to_op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        assert_eq!(
            to_op.read("target/docs/a.txt").await.unwrap().to_vec(),
            b"a"
        );
        assert_eq!(
            to_op
                .read("target/docs/nested/b.txt")
                .await
                .unwrap()
                .to_vec(),
            b"b"
        );
        assert!(to_op.exists("target/docs/empty/").await.unwrap());
        assert!(from_op.exists("docs/a.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_transfer_entries_moves_directory_recursively_and_deletes_source() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "a".as_bytes()).await.unwrap();
        op.write("docs/nested/b.txt", "b".as_bytes()).await.unwrap();
        create_directory(&op, "target").await.unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Move,
            true,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        assert_eq!(op.read("target/docs/a.txt").await.unwrap().to_vec(), b"a");
        assert_eq!(
            op.read("target/docs/nested/b.txt").await.unwrap().to_vec(),
            b"b"
        );
        assert!(!op.exists("docs/a.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_transfer_entries_overwrites_directory_and_removes_stale_children() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "new".as_bytes()).await.unwrap();
        op.write("target/docs/stale.txt", "old".as_bytes())
            .await
            .unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Overwrite,
        )
        .await
        .unwrap();

        assert_eq!(op.read("target/docs/a.txt").await.unwrap().to_vec(), b"new");
        assert!(!op.exists("target/docs/stale.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_transfer_entries_renames_existing_directory_destination() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "new".as_bytes()).await.unwrap();
        op.write("target/docs/old.txt", "old".as_bytes())
            .await
            .unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Rename,
        )
        .await
        .unwrap();

        assert_eq!(
            op.read("target/docs/old.txt").await.unwrap().to_vec(),
            b"old"
        );
        assert_eq!(
            op.read("target/docs copy/a.txt").await.unwrap().to_vec(),
            b"new"
        );
    }

    #[tokio::test]
    async fn test_transfer_entries_skip_existing_directory_leaves_destination_untouched() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "new".as_bytes()).await.unwrap();
        op.write("target/docs/old.txt", "old".as_bytes())
            .await
            .unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Skip,
        )
        .await
        .unwrap();

        assert_eq!(
            op.read("target/docs/old.txt").await.unwrap().to_vec(),
            b"old"
        );
        assert!(!op.exists("target/docs/a.txt").await.unwrap());
        assert_eq!(op.read("docs/a.txt").await.unwrap().to_vec(), b"new");
    }

    #[tokio::test]
    async fn test_transfer_entries_with_progress_cancels_recursive_move_before_source_delete() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "a".as_bytes()).await.unwrap();
        op.write("docs/nested/b.txt", "b".as_bytes()).await.unwrap();
        create_directory(&op, "target").await.unwrap();

        let should_cancel = Arc::new(AtomicBool::new(false));
        let cancel_after_first_progress = Arc::clone(&should_cancel);
        let result = transfer_entries_with_progress(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Move,
            true,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.completed_items > 0 {
                    cancel_after_first_progress.store(true, Ordering::SeqCst);
                }
            },
            || should_cancel.load(Ordering::SeqCst),
        )
        .await;

        assert!(result.is_err());
        assert!(op.exists("docs/a.txt").await.unwrap());
        assert!(op.exists("docs/nested/b.txt").await.unwrap());
        assert!(op.exists("target/docs/").await.unwrap());
    }

    #[tokio::test]
    async fn test_transfer_entries_fail_policy_rejects_duplicate_batch_destinations_before_mutation(
    ) {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op.write("a/file.txt", "a".as_bytes()).await.unwrap();
        from_op.write("b/file.txt", "b".as_bytes()).await.unwrap();
        create_directory(&to_op, "target").await.unwrap();

        let result = transfer_entries(
            &from_op,
            &to_op,
            vec!["a/file.txt".to_string(), "b/file.txt".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await;

        let err = result.expect_err("duplicate destination names must fail before copying");
        assert!(
            matches!(
                err,
                crate::models::CoreError::Storage(ref error)
                    if error.kind() == opendal::ErrorKind::AlreadyExists
            ),
            "unexpected error: {err}"
        );
        assert!(!to_op.exists("target/file.txt").await.unwrap());
        assert_eq!(from_op.read("a/file.txt").await.unwrap().to_vec(), b"a");
        assert_eq!(from_op.read("b/file.txt").await.unwrap().to_vec(), b"b");
    }

    #[tokio::test]
    async fn test_transfer_entries_rejects_folder_copy_into_own_descendant_with_absolute_paths() {
        let op = create_test_operator().await;
        op.create_dir("demo/").await.unwrap();
        op.create_dir("demo/child/").await.unwrap();
        op.write("demo/file.txt", "hello".as_bytes()).await.unwrap();
        op.write("demo/child/existing.txt", "child".as_bytes())
            .await
            .unwrap();

        let result = transfer_entries(
            &op,
            &op,
            vec!["/demo".to_string()],
            "/demo/child",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Fail,
        )
        .await;

        let err = result.expect_err("copying a folder into itself must fail before recursion");
        assert!(
            matches!(
                err,
                crate::models::CoreError::Storage(ref error)
                    if error.kind() == opendal::ErrorKind::IsSameFile
            ),
            "unexpected error: {err}"
        );
        assert!(!op.exists("demo/child/demo/").await.unwrap());
    }

    #[tokio::test]
    async fn test_transfer_entries_skips_existing_destination() {
        let op = create_test_operator().await;
        op.write("source.txt", "new".as_bytes()).await.unwrap();
        create_directory(&op, "target").await.unwrap();
        op.write("target/source.txt", "old".as_bytes())
            .await
            .unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Skip,
        )
        .await
        .unwrap();

        assert_eq!(op.read("source.txt").await.unwrap().to_vec(), b"new");
        assert_eq!(op.read("target/source.txt").await.unwrap().to_vec(), b"old");
    }
}
