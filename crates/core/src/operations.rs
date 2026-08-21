use base64::Engine;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use futures::{StreamExt, TryStreamExt};
use opendal::{ErrorKind, Metadata, Operator};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
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

pub const MAX_TRANSFER_PLAN_ENTRIES: usize = 100_000;

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

#[derive(Debug, Clone, Default)]
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
    let mut out: Vec<Entry> = Vec::new();
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

            if out.len() >= MAX_RECURSIVE_ITEMS as usize {
                out.sort_by(|a, b| a.path.cmp(&b.path));
                return Ok(out);
            }
            out.push(Entry {
                path: entry_path,
                name,
                is_dir,
                size: if is_dir { 0 } else { meta.content_length() },
                modified_at: meta.last_modified().map(|dt| dt.to_string()),
                etag: meta.etag().map(|s| s.to_string()),
            });

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

const MAX_CURSOR_BYTES: usize = 8 * 1024;
const MIN_PAGE_SCAN_BUDGET: usize = 256;
const MAX_PAGE_SCAN_BUDGET: usize = 4_096;
// Cursor replay on backends without `start_after` is bounded by this signed total.
// Keep this aligned with the product's 10k traversal cap for recursive listings.
const MAX_PAGE_TOTAL_SCANNED: usize = MAX_RECURSIVE_ITEMS as usize;
// Non-recursive listings on backends without `start_after` fall back to cursor
// replay; this separate documented maximum bounds that replay. Backends with
// `start_after` continue indefinitely via position-based continuation.
const MAX_PAGE_NON_RECURSIVE_SCANNED: usize = 100_000;
const PAGE_SCAN_MULTIPLIER: usize = 32;

fn page_scan_budget(limit: u32) -> usize {
    (limit as usize)
        .saturating_mul(PAGE_SCAN_MULTIPLIER)
        .clamp(MIN_PAGE_SCAN_BUDGET, MAX_PAGE_SCAN_BUDGET)
}

fn cursor_signing_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut digest = Sha256::new();
        digest.update(uuid::Uuid::new_v4().as_bytes());
        digest.update(uuid::Uuid::new_v4().as_bytes());
        digest.finalize().into()
    })
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_BYTES: usize = 64;
    let mut normalized = [0u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; BLOCK_BYTES];
    let mut outer_pad = [0x5cu8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn encode_page_cursor(cursor: &PageCursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)?;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes);
    let signature = hmac_sha256(cursor_signing_key(), payload.as_bytes());
    Ok(format!(
        "{payload}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_page_cursor(
    encoded: &str,
    path: &str,
    recursive: bool,
    revision: u64,
    max_scanned: usize,
) -> Result<PageCursor> {
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err(crate::models::CoreError::Config(
            "invalid list cursor".to_string(),
        ));
    }
    let (payload, encoded_signature) = encoded
        .split_once('.')
        .ok_or_else(|| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .map_err(|_| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    let expected = hmac_sha256(cursor_signing_key(), payload.as_bytes());
    if !constant_time_eq(&signature, &expected) {
        return Err(crate::models::CoreError::Config(
            "invalid list cursor".to_string(),
        ));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    let cursor: PageCursor = serde_json::from_slice(&bytes)
        .map_err(|_| crate::models::CoreError::Config("invalid list cursor".to_string()))?;
    let normalized_path = normalize_list_path(path);
    let valid_position = cursor.position.as_ref().is_none_or(|position| {
        position.len() <= 4096
            && (normalized_path.is_empty() || position.starts_with(&normalized_path))
            && (recursive
                || !position
                    .trim_start_matches(&normalized_path)
                    .trim_end_matches('/')
                    .contains('/'))
    });
    if cursor.version != 2
        || cursor.path != normalized_path
        || cursor.recursive != recursive
        || cursor.revision != revision
        || !valid_position
        || cursor.scanned > max_scanned
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

async fn entry_from_listed(op: &Operator, object: opendal::Entry) -> Result<Entry> {
    let (full_path, listed) = object.into_parts();
    // OpenDAL list metadata is authoritative when it identifies the entry and gives a
    // non-zero file length. A zero-length file is ambiguous (empty vs. unavailable list
    // metadata), so stat only that narrow case to preserve exact sizes.
    let needs_stat = listed.mode() == opendal::EntryMode::Unknown
        || (listed.mode().is_file() && listed.content_length() == 0);
    let metadata = if needs_stat {
        match op.stat(&full_path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Metadata::new(opendal::EntryMode::from_path(&full_path))
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        listed
    };
    let is_dir = metadata.is_dir();
    Ok(Entry {
        path: if is_dir {
            ensure_dir_path(&full_path)
        } else {
            full_path.clone()
        },
        name: extract_filename(&full_path),
        is_dir,
        size: if is_dir { 0 } else { metadata.content_length() },
        modified_at: metadata.last_modified().map(|dt| dt.to_string()),
        etag: metadata.etag().map(ToOwned::to_owned),
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
    list_entries_page_with_filter(op, path, limit, cursor, recursive, revision, |_| Ok(true)).await
}

/// List a bounded page while applying a path filter before entries are exposed.
///
/// Cursor progress counts every examined backend entry, including filtered entries, so a
/// denied object cannot be recovered by changing page size or replaying a cursor. The filter is
/// evaluated before metadata is returned to the caller.
pub async fn list_entries_page_with_filter<F>(
    op: &Operator,
    path: &str,
    limit: u32,
    encoded_cursor: Option<String>,
    recursive: bool,
    revision: u64,
    filter: F,
) -> Result<ListEntriesPage>
where
    F: Fn(&str) -> Result<bool>,
{
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(crate::models::CoreError::Config(format!(
            "list limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    let max_scanned = if recursive {
        MAX_PAGE_TOTAL_SCANNED
    } else if op.info().capability().list_with_start_after {
        usize::MAX
    } else {
        MAX_PAGE_NON_RECURSIVE_SCANNED
    };
    let cursor = match encoded_cursor.as_deref() {
        Some(cursor) => decode_page_cursor(cursor, path, recursive, revision, max_scanned)?,
        None => PageCursor {
            version: 2,
            path: normalize_list_path(path),
            recursive,
            revision,
            scanned: 0,
            position: None,
        },
    };
    let supports_start_after = op.info().capability().list_with_start_after;
    // Recursive listings keep the cumulative 10k scan cap. Non-recursive
    // listings continue past it via start_after when the backend supports it;
    // otherwise cursor replay is bounded by a separate documented maximum.
    let scan_ceiling = if recursive {
        Some(MAX_PAGE_TOTAL_SCANNED)
    } else if supports_start_after {
        None
    } else {
        Some(MAX_PAGE_NON_RECURSIVE_SCANNED)
    };
    let resume_position = supports_start_after
        .then_some(cursor.position.as_deref())
        .flatten();
    let skip_count = if supports_start_after {
        0
    } else {
        cursor.scanned
    };
    let p = normalize_list_path(path);
    let mut lister = page_lister(op, &p, recursive, resume_position).await?;
    let mut skipped = 0usize;
    let mut scanned = cursor.scanned;
    let mut position = cursor.position.clone();
    let mut objects = Vec::with_capacity(limit as usize);
    let mut has_more = false;
    let mut capped = false;
    let scan_budget = page_scan_budget(limit);
    let mut examined_this_page = 0usize;

    while let Some(object) = lister.try_next().await? {
        let full_path = object.path().to_string();
        if full_path.trim_end_matches('/').is_empty() {
            continue;
        }
        if skipped < skip_count {
            skipped += 1;
            continue;
        }
        // On backends without start-after, replayed cursor entries are unavoidable;
        // budget only newly examined entries so forward progress remains possible.
        examined_this_page = examined_this_page.saturating_add(1);
        if examined_this_page > scan_budget {
            capped = true;
            has_more = true;
            break;
        }
        if let Some(ceiling) = scan_ceiling {
            if scanned >= ceiling {
                capped = true;
                // The signed cursor's cumulative scan ceiling also bounds replay on
                // backends without start-after. Never issue a cursor beyond it.
                has_more = false;
                break;
            }
        }

        let previous_scanned = scanned;
        let previous_position = position.clone();
        scanned = scanned.saturating_add(1);
        position = Some(full_path.clone());
        if !filter(full_path.trim_end_matches('/'))? {
            continue;
        }
        if objects.len() == limit as usize {
            // Leave this allowed object for the next page. Denied entries examined during the
            // look-ahead are deliberately replayed; they remain filtered and never leak.
            scanned = previous_scanned;
            position = previous_position;
            has_more = true;
            break;
        }
        objects.push(object);
    }
    if skipped < skip_count {
        return Err(crate::models::CoreError::Config(
            "invalid list cursor position".to_string(),
        ));
    }

    let entries = futures::stream::iter(
        objects
            .into_iter()
            .map(|object| entry_from_listed(op, object)),
    )
    .buffered(16)
    .try_collect::<Vec<_>>()
    .await?;
    Ok(ListEntriesPage {
        entries,
        next_cursor: has_more
            .then(|| next_page_cursor(path, recursive, revision, scanned, position))
            .transpose()?,
        truncated: capped,
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

    if !op.info().capability().write_with_user_metadata {
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

/// Whether a cleanup failure is only because the target never existed.
fn is_not_found_cleanup(error: &crate::models::CoreError) -> bool {
    matches!(
        error,
        crate::models::CoreError::Storage(e) if e.kind() == ErrorKind::NotFound
    )
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

/// The exact destination a transfer plan produces for a single source path when
/// moving or copying it into `target_dir`, including appending the source
/// basename. Mirrors the destination computation used by the transfer planner so
/// namespace-conflict checks can validate the real destination rather than the
/// bare target directory.
pub fn transfer_destination_path(source_path: &str, target_dir: &str) -> String {
    let normalized = normalize_opendal_path(source_path);
    let name = extract_filename(&normalized);
    join_target_dir(target_dir, &name)
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
async fn collect_transfer_plan_entries<C>(
    from_op: &Operator,
    to_op: &Operator,
    source_path: &str,
    destination_path: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
    entries: &mut Vec<TransferPlanEntry>,
    is_cancelled: &C,
) -> Result<()>
where
    C: Fn() -> bool + ?Sized,
{
    ensure_not_cancelled(is_cancelled)?;
    if entries.len() >= MAX_TRANSFER_PLAN_ENTRIES {
        return Err(crate::models::CoreError::Config(format!(
            "transfer plan exceeds the maximum of {MAX_TRANSFER_PLAN_ENTRIES} entries"
        )));
    }
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
        ensure_not_cancelled(is_cancelled)?;
        let mut lister = from_op.lister(&from_base).await?;
        while let Some(obj) = lister.try_next().await? {
            ensure_not_cancelled(is_cancelled)?;
            if entries.len() >= MAX_TRANSFER_PLAN_ENTRIES {
                return Err(crate::models::CoreError::Config(format!(
                    "transfer plan exceeds the maximum of {MAX_TRANSFER_PLAN_ENTRIES} entries"
                )));
            }
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

async fn estimate_transfer_totals<C>(
    op: &Operator,
    paths: &[String],
    is_cancelled: &C,
) -> Result<(u64, u64)>
where
    C: Fn() -> bool + ?Sized,
{
    let mut total_items = 0_u64;
    let mut total_bytes = 0_u64;

    for path in paths {
        ensure_not_cancelled(is_cancelled)?;
        let remaining = MAX_TRANSFER_PLAN_ENTRIES.saturating_sub(total_items as usize);
        let (items, bytes) = estimate_path_totals(op, path, remaining, is_cancelled).await?;
        total_items = total_items.saturating_add(items);
        total_bytes = total_bytes.saturating_add(bytes);
    }

    Ok((total_items, total_bytes))
}

async fn estimate_path_totals<C>(
    op: &Operator,
    path: &str,
    max_entries: usize,
    is_cancelled: &C,
) -> Result<(u64, u64)>
where
    C: Fn() -> bool + ?Sized,
{
    ensure_not_cancelled(is_cancelled)?;
    if max_entries == 0 {
        return Err(crate::models::CoreError::Config(format!(
            "transfer estimate exceeds the maximum of {MAX_TRANSFER_PLAN_ENTRIES} entries"
        )));
    }
    let meta = stat_for_transfer(op, path).await?;
    if !meta.is_dir() {
        return Ok((1, meta.content_length()));
    }

    let mut total_items = 0_u64;
    let mut total_bytes = 0_u64;
    let mut stack = vec![ensure_dir_path(path)];

    while let Some(dir) = stack.pop() {
        ensure_not_cancelled(is_cancelled)?;
        let mut lister = op.lister(&dir).await?;
        while let Some(obj) = lister.try_next().await? {
            ensure_not_cancelled(is_cancelled)?;
            let child_path = obj.path().to_string();
            if is_current_dir_marker(&dir, &child_path) {
                continue;
            }
            let child_meta = stat_for_transfer(op, &child_path).await?;
            if child_meta.is_dir() {
                stack.push(ensure_dir_path(&child_path));
            } else {
                total_items = total_items.saturating_add(1);
                if total_items as usize > max_entries {
                    return Err(crate::models::CoreError::Config(format!(
                        "transfer estimate exceeds the maximum of {MAX_TRANSFER_PLAN_ENTRIES} entries"
                    )));
                }
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
                    Ok(_) => {}
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
                    Ok(_) => {
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

/// Copy a directory into a sibling staging path and commit it via rename.
///
/// The staging path is a sibling of `to_dir` that did not exist before, so a
/// failed staging or rename never corrupts an existing destination. When the
/// rename commits, the staged tree atomically becomes the destination for
/// backends that support it.
#[allow(clippy::too_many_arguments)]
async fn transactional_create_transfer<P, C>(
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
    let destination = to_dir.trim_end_matches('/');
    let suffix = uuid::Uuid::new_v4();
    let staged = format!("{destination}.infimount-transfer-stage-{suffix}");
    let staged_path = ensure_dir_path(&staged);

    let staged_result = transfer_dir_recursive_with_progress(
        from_op,
        to_op,
        from_dir,
        &staged_path,
        TransferOperation::Copy,
        same_source,
        state,
        progress,
        is_cancelled,
    )
    .await;

    if let Err(error) = staged_result {
        return match delete_recursive(to_op, &staged_path).await {
            Ok(()) => Err(error),
            Err(cleanup) if is_not_found_cleanup(&cleanup) => Err(error),
            Err(_) => Err(crate::models::CoreError::TransferCleanupRequired),
        };
    }

    if is_cancelled() {
        return match delete_recursive(to_op, &staged_path).await {
            Ok(()) => Err(crate::models::CoreError::Config(
                "transfer cancelled".to_string(),
            )),
            Err(cleanup) if is_not_found_cleanup(&cleanup) => Err(
                crate::models::CoreError::Config("transfer cancelled".to_string()),
            ),
            Err(_) => Err(crate::models::CoreError::TransferCleanupRequired),
        };
    }

    if let Err(error) = to_op.rename(&staged, to_dir.trim_end_matches('/')).await {
        return match delete_recursive(to_op, &staged_path).await {
            Ok(()) => Err(error.into()),
            Err(cleanup) if is_not_found_cleanup(&cleanup) => Err(error.into()),
            Err(_) => Err(crate::models::CoreError::TransferCleanupRequired),
        };
    }

    if operation == TransferOperation::Move {
        ensure_not_cancelled(is_cancelled)?;
        delete_recursive(from_op, from_dir).await?;
    }

    Ok(())
}

/// Transfer a directory into a transaction-created destination.
///
/// The top-level destination is guarded to not pre-exist: a destination that
/// existed before the operation is never deleted. Backends with rename support
/// stage the whole tree and commit with a rename; other backends copy directly
/// and remove the destination on failure or cancellation. If cleanup fails the
/// error is `CoreError::TransferCleanupRequired`, which reports
/// `partialDestination: true, cleanupRequired: true` without exposing a local
/// absolute root or credentials.
#[allow(clippy::too_many_arguments)]
async fn transfer_dir_transactional<P, C>(
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
    let to_root = ensure_dir_path(to_dir);

    if path_exists_for_transfer(to_op, &to_root).await? {
        return Err(opendal::Error::new(
            ErrorKind::AlreadyExists,
            "Destination directory already exists",
        )
        .into());
    }

    if to_op.info().capability().rename {
        return transactional_create_transfer(
            from_op,
            to_op,
            from_dir,
            &to_root,
            operation,
            same_source,
            state,
            progress,
            is_cancelled,
        )
        .await;
    }

    let result = transfer_dir_recursive_with_progress(
        from_op,
        to_op,
        from_dir,
        &to_root,
        TransferOperation::Copy,
        same_source,
        state,
        progress,
        is_cancelled,
    )
    .await;

    match result {
        Ok(()) => {
            if operation == TransferOperation::Move {
                ensure_not_cancelled(is_cancelled)?;
                delete_recursive(from_op, from_dir).await?;
            }
            Ok(())
        }
        Err(error) => match delete_recursive(to_op, &to_root).await {
            Ok(()) => Err(error),
            Err(cleanup) if is_not_found_cleanup(&cleanup) => Err(error),
            Err(_) => Err(crate::models::CoreError::TransferCleanupRequired),
        },
    }
}
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
    plan_transfer_entries_cancellable(
        from_op,
        to_op,
        paths,
        target_dir,
        operation,
        same_source,
        conflict_policy,
        || false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn plan_transfer_entries_cancellable<C>(
    from_op: &Operator,
    to_op: &Operator,
    paths: Vec<String>,
    target_dir: &str,
    operation: TransferOperation,
    same_source: bool,
    conflict_policy: TransferConflictPolicy,
    is_cancelled: C,
) -> Result<TransferPlan>
where
    C: Fn() -> bool,
{
    let (paths, target_dir) = normalize_transfer_inputs(paths, target_dir);
    let target_dir = target_dir.as_str();
    let mut entries = Vec::new();

    for source_path in &paths {
        ensure_not_cancelled(&is_cancelled)?;
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
            &is_cancelled,
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
                    return transactional_overwrite_transfer(
                        from_op,
                        to_op,
                        &normalized_src,
                        &normalized_dest,
                        true,
                        operation,
                        &|| false,
                    )
                    .await;
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

        transfer_dir_transactional(
            from_op,
            to_op,
            &normalized_src,
            &normalized_dest,
            operation,
            same_source,
            &mut TransferProgressState::default(),
            &mut |_| {},
            &|| false,
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
                    return transactional_overwrite_transfer(
                        from_op,
                        to_op,
                        &source_path,
                        &destination_path,
                        false,
                        operation,
                        &|| false,
                    )
                    .await;
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

async fn stage_transfer_source<C>(
    from_op: &Operator,
    to_op: &Operator,
    source_path: &str,
    staged_path: &str,
    is_dir: bool,
    is_cancelled: &C,
) -> Result<()>
where
    C: Fn() -> bool + Sync,
{
    ensure_not_cancelled(is_cancelled)?;
    if !is_dir {
        ensure_parent_dir(to_op, staged_path).await?;
        return copy_file_across_operators_with_progress(
            from_op,
            to_op,
            source_path,
            staged_path,
            None::<fn(u64)>,
            Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
        )
        .await;
    }

    let source_root = ensure_dir_path(source_path);
    let staged_root = ensure_dir_path(staged_path);
    to_op.create_dir(&staged_root).await?;
    let mut stack = vec![(source_root, staged_root)];
    while let Some((source_dir, target_dir)) = stack.pop() {
        ensure_not_cancelled(is_cancelled)?;
        let mut lister = from_op.lister(&source_dir).await?;
        while let Some(entry) = lister.try_next().await? {
            ensure_not_cancelled(is_cancelled)?;
            let child = entry.path().to_string();
            if is_current_dir_marker(&source_dir, &child) {
                continue;
            }
            let metadata = stat_for_transfer(from_op, &child).await?;
            let destination = join_target_dir(&target_dir, &extract_filename(&child));
            if metadata.is_dir() {
                let source_child = ensure_dir_path(&child);
                let destination_child = ensure_dir_path(&destination);
                to_op.create_dir(&destination_child).await?;
                stack.push((source_child, destination_child));
            } else {
                copy_file_across_operators_with_progress(
                    from_op,
                    to_op,
                    &child,
                    &destination,
                    None::<fn(u64)>,
                    Some(is_cancelled as &(dyn Fn() -> bool + Sync)),
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn transactional_overwrite_transfer<C>(
    from_op: &Operator,
    to_op: &Operator,
    source_path: &str,
    destination_path: &str,
    is_dir: bool,
    operation: TransferOperation,
    is_cancelled: &C,
) -> Result<()>
where
    C: Fn() -> bool + Sync,
{
    let capabilities = to_op.info().capability();
    if !capabilities.rename || !capabilities.delete {
        return Err(crate::models::CoreError::Config(
            "safe transactional overwrite is unsupported by this backend".to_string(),
        ));
    }
    let normalized_destination = destination_path.trim_end_matches('/');
    let suffix = uuid::Uuid::new_v4();
    let staged = format!("{normalized_destination}.infimount-transfer-stage-{suffix}");
    let backup = format!("{normalized_destination}.infimount-transfer-backup-{suffix}");
    let staged_path = if is_dir {
        ensure_dir_path(&staged)
    } else {
        staged.clone()
    };
    let backup_path = if is_dir {
        ensure_dir_path(&backup)
    } else {
        backup.clone()
    };

    if let Err(error) = stage_transfer_source(
        from_op,
        to_op,
        source_path,
        &staged_path,
        is_dir,
        is_cancelled,
    )
    .await
    {
        let cleanup = delete_recursive(to_op, &staged_path).await;
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(crate::models::CoreError::Config(format!(
                "{error}; staged transfer cleanup failed at {staged_path}: {cleanup_error}"
            ))),
        };
    }
    if is_cancelled() {
        delete_recursive(to_op, &staged_path)
            .await
            .map_err(|cleanup| {
                crate::models::CoreError::Config(format!(
                "transfer cancelled but staged destination is preserved at {staged_path}: {cleanup}"
            ))
            })?;
        return Err(crate::models::CoreError::Config(
            "transfer cancelled".to_string(),
        ));
    }
    if let Err(error) = to_op.rename(normalized_destination, &backup).await {
        delete_recursive(to_op, &staged_path).await.map_err(|cleanup| {
            crate::models::CoreError::Config(format!(
                "destination preservation failed ({error}); staged destination is preserved at {staged_path}: {cleanup}"
            ))
        })?;
        return Err(error.into());
    }
    if let Err(error) = to_op.rename(&staged, normalized_destination).await {
        to_op.rename(&backup, normalized_destination).await.map_err(|rollback| {
            crate::models::CoreError::Config(format!(
                "transfer commit failed ({error}); rollback failed and previous destination is preserved at {backup_path}: {rollback}"
            ))
        })?;
        delete_recursive(to_op, &staged_path).await.map_err(|cleanup| {
            crate::models::CoreError::Config(format!(
                "transfer commit failed ({error}) and staged destination remains at {staged_path}: {cleanup}"
            ))
        })?;
        return Err(error.into());
    }

    if let Err(error) = delete_recursive(to_op, &backup_path).await {
        delete_recursive(to_op, destination_path).await.map_err(|rollback| {
            crate::models::CoreError::Config(format!(
                "transfer backup cleanup failed ({error}); new destination cleanup failed and previous destination remains at {backup_path}: {rollback}"
            ))
        })?;
        to_op.rename(&backup, normalized_destination).await.map_err(|rollback| {
            crate::models::CoreError::Config(format!(
                "transfer backup cleanup failed ({error}); rollback failed and previous destination is preserved at {backup_path}: {rollback}"
            ))
        })?;
        return Err(error);
    }
    if operation == TransferOperation::Move {
        ensure_not_cancelled(is_cancelled)?;
        delete_recursive(from_op, source_path).await?;
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
                        transactional_overwrite_transfer(
                            from_op,
                            to_op,
                            &from_path,
                            &dest_dir,
                            true,
                            operation,
                            &|| false,
                        )
                        .await?;
                        continue;
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

            transfer_dir_transactional(
                from_op,
                to_op,
                &ensure_dir_path(&from_path),
                &dest_dir,
                operation,
                same_source,
                &mut TransferProgressState::default(),
                &mut |_| {},
                &|| false,
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
                        transactional_overwrite_transfer(
                            from_op,
                            to_op,
                            &from_path,
                            &dest_file,
                            false,
                            operation,
                            &|| false,
                        )
                        .await?;
                        continue;
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
    let (total_items, total_bytes) =
        estimate_transfer_totals(from_op, &paths, &is_cancelled).await?;
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
                        transactional_overwrite_transfer(
                            from_op,
                            to_op,
                            &from_path,
                            &dest_dir,
                            true,
                            operation,
                            &is_cancelled,
                        )
                        .await?;
                        state.completed_items = state.completed_items.saturating_add(1);
                        emit_progress(&mut progress, &state, &from_path);
                        continue;
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

            transfer_dir_transactional(
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
                        transactional_overwrite_transfer(
                            from_op,
                            to_op,
                            &from_path,
                            &dest_file,
                            false,
                            operation,
                            &is_cancelled,
                        )
                        .await?;
                        state.completed_items = state.completed_items.saturating_add(1);
                        state.bytes_transferred = state
                            .bytes_transferred
                            .saturating_add(meta.content_length());
                        emit_progress(&mut progress, &state, &from_path);
                        continue;
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

/// Stream one historical object version to an exact local path without materializing it in IPC.
pub async fn download_file_version_to_local_path(
    op: &Operator,
    source_path: &str,
    version: &str,
    local_path: &Path,
) -> Result<u64> {
    let metadata = op.stat_with(source_path).version(version).await?;
    if metadata.is_dir() {
        return Err(crate::models::CoreError::Config(
            "cannot download a directory version as a file".to_string(),
        ));
    }
    let expected_bytes = metadata.content_length();
    let mut reader = op
        .reader_with(source_path)
        .version(version)
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
        if transferred != expected_bytes || fs::metadata(local_path).await?.len() != expected_bytes
        {
            return Err(crate::models::CoreError::Config(
                "version download byte verification failed".to_string(),
            ));
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
    upload_local_file_to_path_cancellable(op, source_path, target_path, || false).await
}

/// Stream a local staging file with periodic cancellation and failure-safe overwrite handling.
///
/// Backends with rename support receive the upload under a unique sibling name first. Existing
/// destinations are moved aside until the staged object has been committed and verified. A
/// backend that cannot rename may only create a previously absent destination; overwriting on
/// such a backend is refused rather than risking irreversible loss of the old object.
async fn restore_preserved_backup(
    op: &Operator,
    backup_path: &str,
    target_path: &str,
    remove_current: bool,
    context: &str,
) -> Result<()> {
    if remove_current {
        op.delete(target_path).await.map_err(|error| {
            crate::models::CoreError::Config(format!(
                "{context}; rollback could not remove the incomplete destination; previous object is preserved at {backup_path}: {error}"
            ))
        })?;
    }
    op.rename(backup_path, target_path).await.map_err(|error| {
        crate::models::CoreError::Config(format!(
            "{context}; rollback failed and the previous object is preserved at {backup_path}: {error}"
        ))
    })
}

pub async fn upload_local_file_to_path_cancellable<F>(
    op: &Operator,
    source_path: &Path,
    target_path: &str,
    cancelled: F,
) -> Result<()>
where
    F: Fn() -> bool + Send + Sync,
{
    let expected_bytes = fs::metadata(source_path).await?.len();
    let target_exists = match op.stat(target_path).await {
        Ok(metadata) => {
            if metadata.is_dir() {
                return Err(crate::models::CoreError::Config(
                    "upload target is a directory".to_string(),
                ));
            }
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let capabilities = op.info().capability();
    if !capabilities.rename {
        if target_exists {
            return Err(crate::models::CoreError::Config(
                "safe overwrite is unsupported by this backend".to_string(),
            ));
        }
        return stream_local_file_cancellable(op, source_path, target_path, &cancelled, true).await;
    }

    let suffix = uuid::Uuid::new_v4();
    let staged_path = format!("{target_path}.infimount-upload-{suffix}");
    let backup_path = format!("{target_path}.infimount-backup-{suffix}");
    stream_local_file_cancellable(op, source_path, &staged_path, &cancelled, true).await?;
    if cancelled() {
        op.delete(&staged_path).await.map_err(|error| {
            crate::models::CoreError::Config(format!(
                "upload cancelled but staged object is preserved at {staged_path} because cleanup failed: {error}"
            ))
        })?;
        return Err(crate::models::CoreError::Config(
            "upload cancelled".to_string(),
        ));
    }

    let mut old_moved = false;
    if target_exists {
        if let Err(error) = op.rename(target_path, &backup_path).await {
            let _ = op.delete(&staged_path).await;
            return Err(error.into());
        }
        old_moved = true;
    }

    if cancelled() {
        if old_moved {
            restore_preserved_backup(
                op,
                &backup_path,
                target_path,
                false,
                "upload cancelled after preserving the previous destination",
            )
            .await?;
        }
        op.delete(&staged_path).await.map_err(|error| {
            crate::models::CoreError::Config(format!(
                "upload cancelled but staged-object cleanup failed at {staged_path}: {error}"
            ))
        })?;
        return Err(crate::models::CoreError::Config(
            "upload cancelled".to_string(),
        ));
    }

    if let Err(error) = op.rename(&staged_path, target_path).await {
        if old_moved {
            restore_preserved_backup(
                op,
                &backup_path,
                target_path,
                false,
                "upload commit failed after preserving the previous destination",
            )
            .await?;
        }
        op.delete(&staged_path).await.map_err(|cleanup_error| {
            crate::models::CoreError::Config(format!(
                "upload commit failed ({error}); staged object is preserved at {staged_path} because cleanup failed: {cleanup_error}"
            ))
        })?;
        return Err(error.into());
    }

    let verification = op.stat(target_path).await;
    let verification_error = match verification {
        Ok(metadata) if metadata.content_length() == expected_bytes => None,
        Ok(metadata) => Some(crate::models::CoreError::Config(format!(
            "upload verification failed: expected {expected_bytes}, found {}",
            metadata.content_length()
        ))),
        Err(error) => Some(error.into()),
    };
    if cancelled() || verification_error.is_some() {
        let failure = if cancelled() {
            crate::models::CoreError::Config("upload cancelled".to_string())
        } else {
            verification_error.expect("verification error was checked")
        };
        if old_moved {
            restore_preserved_backup(
                op,
                &backup_path,
                target_path,
                true,
                "upload verification or cancellation failed",
            )
            .await?;
        } else {
            op.delete(target_path).await.map_err(|cleanup_error| {
                crate::models::CoreError::Config(format!(
                    "{failure}; incomplete destination cleanup failed at {target_path}: {cleanup_error}"
                ))
            })?;
        }
        return Err(failure);
    }

    if old_moved {
        if let Err(error) = op.delete(&backup_path).await {
            restore_preserved_backup(
                op,
                &backup_path,
                target_path,
                true,
                "upload backup cleanup failed",
            )
            .await?;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn stream_local_file_cancellable<F>(
    op: &Operator,
    source_path: &Path,
    target_path: &str,
    cancelled: &F,
    cleanup_on_error: bool,
) -> Result<()>
where
    F: Fn() -> bool + Send + Sync,
{
    let result = async {
        let expected_bytes = fs::metadata(source_path).await?.len();
        let mut source = fs::File::open(source_path).await?;
        let mut destination = op.writer(target_path).await?.into_futures_async_write();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut transferred_bytes = 0_u64;
        loop {
            if cancelled() {
                return Err(crate::models::CoreError::Config("upload cancelled".to_string()));
            }
            let read = tokio::io::AsyncReadExt::read(&mut source, &mut buffer).await?;
            if read == 0 {
                break;
            }
            destination.write_all(&buffer[..read]).await?;
            transferred_bytes = transferred_bytes.saturating_add(read as u64);
        }
        if cancelled() {
            return Err(crate::models::CoreError::Config("upload cancelled".to_string()));
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
    .await;
    if result.is_err() && cleanup_on_error {
        let _ = op.delete(target_path).await;
    }
    result
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

        upload_local_file_to_path(op, src, &target_path).await?;
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
                    upload_local_file_to_path(op, &child_path, &target_path).await?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub truncated: bool,
}

/// Maximum number of versions a single version-listing scan may collect.
pub const MAX_VERSIONS_SCANNED: usize = 10_000;
/// Maximum page size for a single version-listing request.
pub const MAX_VERSIONS_PAGE: u32 = 1_000;

#[derive(Debug, Serialize, Deserialize)]
struct VersionCursor {
    version: u8,
    storage_id: String,
    path: String,
    revision: u64,
    offset: usize,
    scan_cap: usize,
}

/// Bounded version listing shared by the desktop and the MCP server.
///
/// The cursor is HMAC-signed, bound to the storage id, the normalized path and
/// the storage revision, and never continues beyond the scan cap.
pub async fn list_file_versions_page(
    op: &Operator,
    storage_id: &str,
    path: &str,
    limit: u32,
    cursor: Option<&str>,
    revision: u64,
) -> Result<ListVersionsResult> {
    if limit == 0 || limit > MAX_VERSIONS_PAGE {
        return Err(crate::models::CoreError::Config(format!(
            "version limit must be between 1 and {MAX_VERSIONS_PAGE}"
        )));
    }
    let normalized = normalize_opendal_path(path);
    let offset = decode_version_cursor(cursor, storage_id, &normalized, revision)?;

    let mut lister = match op.lister_with(&normalized).versions(true).await {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok(ListVersionsResult {
                path: normalized,
                versions: vec![],
                next_cursor: None,
                truncated: false,
            });
        }
        Err(e) => return Err(e.into()),
    };

    let mut collected = Vec::new();
    let mut scanned = 0usize;
    let mut hit_cap = false;
    while let Some(entry) = lister.try_next().await? {
        let meta = entry.metadata();
        let Some(version) = meta.version() else {
            continue;
        };
        scanned = scanned.saturating_add(1);
        if scanned > MAX_VERSIONS_SCANNED {
            hit_cap = true;
            break;
        }
        if version == "default" {
            continue;
        }
        collected.push(FileVersion {
            version: version.to_string(),
            size_bytes: Some(meta.content_length()),
            modified_at: meta.last_modified().map(|dt| dt.to_string()),
            etag: meta.etag().map(|s| s.to_string()),
        });
    }

    collected.sort_by(|a, b| {
        let a_time = a.modified_at.as_deref().unwrap_or("");
        let b_time = b.modified_at.as_deref().unwrap_or("");
        b_time.cmp(a_time).then_with(|| a.version.cmp(&b.version))
    });

    let start = offset.min(collected.len());
    let end = (start + limit as usize).min(collected.len());
    let page = collected[start..end].to_vec();
    // More versions may exist beyond the scan cap; report truncation without
    // ever issuing a continuation that would scan beyond the cap.
    let next_cursor = if end < collected.len() {
        Some(encode_version_cursor(
            storage_id,
            &normalized,
            revision,
            end,
        )?)
    } else {
        None
    };

    Ok(ListVersionsResult {
        path: normalized,
        versions: page,
        next_cursor,
        truncated: hit_cap,
    })
}

fn encode_version_cursor(
    storage_id: &str,
    path: &str,
    revision: u64,
    offset: usize,
) -> Result<String> {
    let cursor = VersionCursor {
        version: 1,
        storage_id: storage_id.to_string(),
        path: path.to_string(),
        revision,
        offset,
        scan_cap: MAX_VERSIONS_SCANNED,
    };
    let bytes = serde_json::to_vec(&cursor)?;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes);
    let signature = hmac_sha256(cursor_signing_key(), payload.as_bytes());
    Ok(format!(
        "{payload}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_version_cursor(
    encoded: Option<&str>,
    storage_id: &str,
    path: &str,
    revision: u64,
) -> Result<usize> {
    let Some(encoded) = encoded else {
        return Ok(0);
    };
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err(crate::models::CoreError::Config(
            "invalid version cursor".to_string(),
        ));
    }
    let (payload, encoded_signature) = encoded
        .split_once('.')
        .ok_or_else(|| crate::models::CoreError::Config("invalid version cursor".to_string()))?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .map_err(|_| crate::models::CoreError::Config("invalid version cursor".to_string()))?;
    let expected = hmac_sha256(cursor_signing_key(), payload.as_bytes());
    if !constant_time_eq(&signature, &expected) {
        return Err(crate::models::CoreError::Config(
            "invalid version cursor".to_string(),
        ));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| crate::models::CoreError::Config("invalid version cursor".to_string()))?;
    let cursor: VersionCursor = serde_json::from_slice(&bytes)
        .map_err(|_| crate::models::CoreError::Config("invalid version cursor".to_string()))?;
    if cursor.version != 1
        || cursor.storage_id != storage_id
        || cursor.path != path
        || cursor.revision != revision
        || cursor.scan_cap != MAX_VERSIONS_SCANNED
        || cursor.offset > MAX_VERSIONS_SCANNED
    {
        return Err(crate::models::CoreError::Config(
            "version cursor does not match the current query or storage revision".to_string(),
        ));
    }
    Ok(cursor.offset)
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
        Operator::new(builder).unwrap()
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

    #[test]
    fn version_cursor_is_bound_to_storage_path_and_revision() {
        let cursor = encode_version_cursor("storage-1", "dir/file.txt", 7, 10).unwrap();
        assert_eq!(
            decode_version_cursor(Some(&cursor), "storage-1", "dir/file.txt", 7).unwrap(),
            10
        );

        // Replay on another storage id is rejected.
        assert!(decode_version_cursor(Some(&cursor), "storage-2", "dir/file.txt", 7).is_err());
        // Replay on another path is rejected.
        assert!(decode_version_cursor(Some(&cursor), "storage-1", "dir/other.txt", 7).is_err());
        // Replay after a storage revision change is rejected.
        assert!(decode_version_cursor(Some(&cursor), "storage-1", "dir/file.txt", 8).is_err());
        // An absent cursor starts at the beginning.
        assert_eq!(
            decode_version_cursor(None, "storage-1", "dir/file.txt", 7).unwrap(),
            0
        );
    }

    #[test]
    fn version_cursor_rejects_forged_signature_and_oversized_scan() {
        let cursor = encode_version_cursor("storage-1", "dir/file.txt", 7, 10).unwrap();
        let (payload, signature) = cursor.split_once('.').unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        value["offset"] = serde_json::json!(99);
        let forged_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&value).unwrap());
        let forged = format!("{forged_payload}.{signature}");
        assert!(decode_version_cursor(Some(&forged), "storage-1", "dir/file.txt", 7).is_err());

        // A validly signed cursor beyond the scan cap is rejected.
        let over_cap =
            encode_version_cursor("storage-1", "dir/file.txt", 7, MAX_VERSIONS_SCANNED + 1)
                .unwrap();
        assert!(decode_version_cursor(Some(&over_cap), "storage-1", "dir/file.txt", 7).is_err());

        // A cursor signed for a different scan cap is rejected.
        let mut stale: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        stale["scan_cap"] = serde_json::json!(5_000);
        let stale_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&stale).unwrap());
        let stale_cursor = format!("{stale_payload}.{signature}");
        assert!(
            decode_version_cursor(Some(&stale_cursor), "storage-1", "dir/file.txt", 7).is_err()
        );
    }

    #[test]
    fn version_scan_cap_is_never_exceeded() {
        // The bounded collection loop must stop before exceeding the cap, so the
        // collected page is never larger than the cap even for a full scan.
        let cap = MAX_VERSIONS_SCANNED;
        let mut collected = Vec::with_capacity(cap + 1);
        let mut scanned = 0usize;
        for _ in 0..(cap + 100) {
            scanned = scanned.saturating_add(1);
            if scanned > MAX_VERSIONS_SCANNED {
                break;
            }
            collected.push(FileVersion {
                version: format!("v{scanned}"),
                size_bytes: Some(1),
                modified_at: None,
                etag: None,
            });
        }
        assert_eq!(collected.len(), MAX_VERSIONS_SCANNED);
        assert!(collected.len() <= MAX_VERSIONS_SCANNED);
    }

    #[tokio::test]
    async fn paginated_cursor_rejects_forged_scanned_and_position_state() {
        let op = create_test_operator().await;
        for name in ["a.txt", "b.txt", "c.txt"] {
            op.write(name, name.as_bytes()).await.unwrap();
        }
        let cursor = list_entries_page(&op, "/", 1, None, false, 9)
            .await
            .unwrap()
            .next_cursor
            .unwrap();
        let (payload, signature) = cursor.split_once('.').unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        value["scanned"] = serde_json::json!(50_000);
        value["position"] = serde_json::json!("outside/forged.txt");
        let forged_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&value).unwrap());
        let forged = format!("{forged_payload}.{signature}");
        assert!(list_entries_page(&op, "/", 1, Some(forged), false, 9)
            .await
            .is_err());

        let over_ceiling = encode_page_cursor(&PageCursor {
            version: 2,
            path: String::new(),
            recursive: true,
            revision: 9,
            scanned: MAX_PAGE_TOTAL_SCANNED + 1,
            position: Some("z.txt".to_string()),
        })
        .unwrap();
        assert!(decode_page_cursor(&over_ceiling, "/", true, 9, MAX_PAGE_TOTAL_SCANNED).is_err());
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
    async fn filtered_recursive_pages_are_bounded_and_do_not_expose_denied_paths() {
        let op = create_test_operator().await;
        for index in 0..40 {
            op.write(
                &format!("allowed/nested/{index:03}.txt"),
                b"hello".as_slice(),
            )
            .await
            .unwrap();
            op.write(
                &format!("denied/nested/{index:03}.txt"),
                b"secret".as_slice(),
            )
            .await
            .unwrap();
        }
        let mut cursor = None;
        let mut paths = Vec::new();
        let mut pages = 0usize;
        loop {
            let page = list_entries_page_with_filter(&op, "/", 7, cursor, true, 22, |path| {
                Ok(!path.starts_with("denied"))
            })
            .await
            .unwrap();
            assert!(page.entries.len() <= 7);
            assert!(page
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("denied")));
            paths.extend(page.entries.into_iter().map(|entry| entry.path));
            cursor = page.next_cursor;
            pages += 1;
            assert!(pages < 20, "cursor did not make bounded progress");
            if cursor.is_none() {
                break;
            }
        }
        assert!(paths.iter().any(|path| path == "allowed/nested/039.txt"));
        let unique = paths.iter().collect::<HashSet<_>>();
        assert_eq!(unique.len(), paths.len());
    }

    #[tokio::test]
    async fn denied_heavy_pages_stop_at_scan_budget_and_resume() {
        let op = create_test_operator().await;
        for index in 0..300 {
            op.write(&format!("a-denied-{index:03}.txt"), b"x".as_slice())
                .await
                .unwrap();
        }
        op.write("z-allowed.txt", b"ok".as_slice()).await.unwrap();

        for recursive in [false, true] {
            let first = list_entries_page_with_filter(&op, "/", 1, None, recursive, 44, |path| {
                Ok(path == "z-allowed.txt")
            })
            .await
            .unwrap();
            assert!(first.entries.is_empty());
            assert!(first.truncated, "scan budget exhaustion must be explicit");
            if let Some(cursor) = first.next_cursor {
                let second = list_entries_page_with_filter(
                    &op,
                    "/",
                    1,
                    Some(cursor),
                    recursive,
                    44,
                    |path| Ok(path == "z-allowed.txt"),
                )
                .await
                .unwrap();
                assert_eq!(second.entries.len(), 1);
                assert_eq!(second.entries[0].path, "z-allowed.txt");
            } else {
                assert!(first.truncated);
            }
        }
    }

    #[tokio::test]
    async fn filtered_recursive_pagination_stops_at_signed_total_scan_ceiling() {
        let op = create_test_operator().await;
        for index in 0..=MAX_PAGE_TOTAL_SCANNED {
            op.write(&format!("denied-{index:05}.txt"), b"x".as_slice())
                .await
                .unwrap();
        }

        let mut cursor = None;
        let mut pages = 0usize;
        loop {
            let page =
                list_entries_page_with_filter(&op, "/", MAX_LIST_LIMIT, cursor, true, 45, |_| {
                    Ok(false)
                })
                .await
                .unwrap();
            assert!(page.entries.is_empty());
            assert!(page.truncated);
            pages += 1;
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
            assert!(pages < 4, "scan ceiling did not bound cursor replay");
        }
        assert_eq!(pages, 3);
    }

    #[tokio::test]
    async fn paged_listing_uses_metadata_returned_by_the_lister() {
        let op = create_test_operator().await;
        op.write("sized.txt", b"hello".as_slice()).await.unwrap();
        let page = list_entries_page(&op, "/", 10, None, false, 1)
            .await
            .unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == "sized.txt")
            .unwrap();
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 5);
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
        assert!(!result.truncated);

        let eof = read_file_range(&op, "file", 5, 1).await.unwrap();
        assert!(eof.bytes.is_empty());
        assert!(!eof.truncated);
        assert!(read_file_range(&op, "file", u64::MAX, 1).await.is_err());
        assert!(read_file_range(&op, "file", u64::MAX - 1, 0).await.is_err());
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

    #[tokio::test]
    async fn cancellable_staged_upload_removes_partial_objects_and_preserves_overwrite() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let root = unique_temp_dir("cancel-upload");
        let source = root.join("source.bin");
        tokio::fs::write(&source, vec![0x5a; 1024 * 1024])
            .await
            .unwrap();
        let storage_root = root.join("storage");
        std::fs::create_dir_all(&storage_root).unwrap();
        let op = Operator::new(Fs::default().root(storage_root.to_str().unwrap())).unwrap();
        op.write("target.bin", "original").await.unwrap();

        let checks = AtomicUsize::new(0);
        let error = upload_local_file_to_path_cancellable(&op, &source, "target.bin", || {
            checks.fetch_add(1, Ordering::SeqCst) >= 2
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(op.read("target.bin").await.unwrap().to_vec(), b"original");
        let names = op
            .list("")
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_string())
            .filter(|name| name != "/")
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["target.bin"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn staged_upload_verifies_exact_bytes_without_orphans() {
        let root = unique_temp_dir("exact-upload");
        let source = root.join("source.bin");
        let content = vec![0xa5; 768 * 1024 + 17];
        tokio::fs::write(&source, &content).await.unwrap();
        let storage_root = root.join("storage");
        std::fs::create_dir_all(&storage_root).unwrap();
        let op = Operator::new(Fs::default().root(storage_root.to_str().unwrap())).unwrap();

        upload_local_file_to_path_cancellable(&op, &source, "target.bin", || false)
            .await
            .unwrap();
        assert_eq!(
            op.stat("target.bin").await.unwrap().content_length(),
            content.len() as u64
        );
        assert_eq!(op.read("target.bin").await.unwrap().to_vec(), content);
        let entries = op
            .list("")
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.name() != "/")
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), "target.bin");
        std::fs::remove_dir_all(root).unwrap();
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
        let op = Operator::new(Fs::default().root(storage_root.to_str().unwrap())).unwrap();
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
        assert!(!op.info().capability().write_with_user_metadata);

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

        let to_op = Operator::new(Fs::default().root(root.to_str().unwrap())).unwrap();

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
    async fn transfer_planning_is_cancellable_and_entry_bounded() {
        let op = create_test_operator().await;
        op.write("source.txt", b"data".as_slice()).await.unwrap();
        let cancelled = plan_transfer_entries_cancellable(
            &op,
            &op,
            vec!["source.txt".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Fail,
            || true,
        )
        .await
        .unwrap_err();
        assert!(matches!(cancelled, crate::models::CoreError::Storage(_)));

        let mut entries = vec![
            TransferPlanEntry {
                source_path: String::new(),
                destination_path: String::new(),
                is_dir: false,
                size: 0,
                action: TransferPlanAction::Create,
            };
            MAX_TRANSFER_PLAN_ENTRIES
        ];
        let bounded = collect_transfer_plan_entries(
            &op,
            &op,
            "source.txt",
            "target/source.txt",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Fail,
            &mut entries,
            &|| false,
        )
        .await
        .unwrap_err();
        assert!(bounded.to_string().contains("maximum"));
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
    async fn test_transfer_entries_refuses_unsafe_overwrite_without_rename() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "new".as_bytes()).await.unwrap();
        op.write("target/docs/stale.txt", "old".as_bytes())
            .await
            .unwrap();

        let error = transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Overwrite,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("safe transactional overwrite"));
        assert_eq!(
            op.read("target/docs/stale.txt").await.unwrap().to_vec(),
            b"old"
        );
        assert!(!op.exists("target/docs/a.txt").await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_transactional_overwrite_file_on_rename_backend() {
        let root = unique_temp_dir("transactional-overwrite");
        let op = Operator::new(Fs::default().root(root.to_str().unwrap())).unwrap();
        op.write("source/a.txt", "new".as_bytes()).await.unwrap();
        op.write("target/a.txt", "old".as_bytes()).await.unwrap();

        transfer_entries(
            &op,
            &op,
            vec!["source/a.txt".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Overwrite,
        )
        .await
        .unwrap();

        assert_eq!(op.read("target/a.txt").await.unwrap().to_vec(), b"new");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn transactional_overwrite_cancellation_preserves_original() {
        let root = unique_temp_dir("transactional-overwrite-cancel");
        let op = Operator::new(Fs::default().root(root.to_str().unwrap())).unwrap();
        op.write("source.bin", vec![7u8; 1024 * 1024])
            .await
            .unwrap();
        op.write("target.bin", b"original".as_slice())
            .await
            .unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let error = transactional_overwrite_transfer(
            &op,
            &op,
            "source.bin",
            "target.bin",
            false,
            TransferOperation::Copy,
            &|| checks.fetch_add(1, Ordering::SeqCst) > 3,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, crate::models::CoreError::Storage(_)));
        assert_eq!(op.read("target.bin").await.unwrap().to_vec(), b"original");
        assert_eq!(
            op.read("source.bin").await.unwrap().to_vec().len(),
            1024 * 1024
        );
        let names = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(names
            .iter()
            .all(|name| !name.contains("infimount-transfer")));
        let _ = fs::remove_dir_all(root);
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
        assert!(!op.exists("target/docs/").await.unwrap());
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

    #[tokio::test]
    async fn cancellation_removes_transaction_created_destination() {
        let from_op = create_test_operator().await;
        let to_op = create_test_operator().await;
        from_op.write("docs/a.txt", "a".as_bytes()).await.unwrap();
        from_op
            .write("docs/nested/b.txt", "b".as_bytes())
            .await
            .unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let set_cancelled = Arc::clone(&cancelled);
        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.completed_items > 0 {
                    set_cancelled.store(true, Ordering::SeqCst);
                }
            },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;

        assert!(result.is_err());
        assert!(!to_op.exists("target/docs/").await.unwrap());
        assert!(!to_op.exists("target/docs/a.txt").await.unwrap());

        let mut lister = to_op.lister("target/").await.unwrap();
        let mut leftover = Vec::new();
        while let Some(obj) = lister.try_next().await.unwrap() {
            leftover.push(obj.path().to_string());
        }
        assert!(
            leftover
                .iter()
                .all(|p| !p.contains("infimount-transfer-stage")),
            "staging leftovers remain: {leftover:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failure_in_middle_of_directory_copy_removes_new_destination() {
        let src_root = unique_temp_dir("mid-fail-src");
        fs::create_dir_all(src_root.join("docs")).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(src_root.join("docs").join(name), name).unwrap();
        }
        let dst_root = unique_temp_dir("mid-fail-dst");
        fs::create_dir_all(dst_root.join("target")).unwrap();

        let from_op = Operator::new(Fs::default().root(src_root.to_str().unwrap())).unwrap();
        let to_op = Operator::new(Fs::default().root(dst_root.to_str().unwrap())).unwrap();

        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_flag = cancelled.clone();
        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.completed_items > 0 {
                    cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            },
            move || cancelled.load(std::sync::atomic::Ordering::SeqCst),
        )
        .await;

        assert!(result.is_err());
        assert!(!to_op.exists("target/docs/").await.unwrap());
        assert!(!dst_root.join("target/docs").exists());

        fs::remove_dir_all(&src_root).unwrap();
        fs::remove_dir_all(&dst_root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_failure_reports_partial_destination() {
        let src_root = unique_temp_dir("cleanup-fail-src");
        fs::create_dir_all(src_root.join("docs")).unwrap();
        for name in ["a.txt", "b.txt"] {
            fs::write(src_root.join("docs").join(name), name).unwrap();
        }
        let dst_root = unique_temp_dir("cleanup-fail-dst");
        let target = dst_root.join("target");
        fs::create_dir_all(&target).unwrap();

        let from_op = Operator::new(Fs::default().root(src_root.to_str().unwrap())).unwrap();
        let to_op = Operator::new(Fs::default().root(dst_root.to_str().unwrap())).unwrap();

        let lock_target = target.clone();
        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.completed_items > 0 {
                    let _ = fs::set_permissions(&lock_target, fs::Permissions::from_mode(0o500));
                }
            },
            || false,
        )
        .await;

        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(matches!(
            result,
            Err(crate::models::CoreError::TransferCleanupRequired)
        ));
        assert!(!to_op.exists("target/docs/").await.unwrap());

        let mut lister = to_op.lister("target/").await.unwrap();
        let mut leftover = Vec::new();
        while let Some(obj) = lister.try_next().await.unwrap() {
            leftover.push(obj.path().to_string());
        }
        assert!(
            leftover
                .iter()
                .any(|p| p.contains("infimount-transfer-stage")),
            "expected a staging leftover requiring manual cleanup"
        );

        fs::remove_dir_all(&src_root).unwrap();
        fs::remove_dir_all(&dst_root).unwrap();
    }

    #[tokio::test]
    async fn existing_destination_is_never_removed_accidentally() {
        let op = create_test_operator().await;
        op.write("docs/a.txt", "new".as_bytes()).await.unwrap();
        create_directory(&op, "target").await.unwrap();
        op.write("target/docs/old.txt", "old".as_bytes())
            .await
            .unwrap();

        let fail_policy = transfer_entries(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Fail,
        )
        .await;
        assert!(fail_policy.is_err());
        assert!(op.exists("target/docs/").await.unwrap());
        assert_eq!(
            op.read("target/docs/old.txt").await.unwrap().to_vec(),
            b"old"
        );

        let cancelled = Arc::new(AtomicBool::new(false));
        let set_cancelled = Arc::clone(&cancelled);
        let overwrite_cancelled = transfer_entries_with_progress(
            &op,
            &op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Copy,
            true,
            TransferConflictPolicy::Overwrite,
            move |progress| {
                if progress.completed_items > 0 {
                    set_cancelled.store(true, Ordering::SeqCst);
                }
            },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;
        assert!(overwrite_cancelled.is_err());
        assert!(op.exists("target/docs/").await.unwrap());
        assert_eq!(
            op.read("target/docs/old.txt").await.unwrap().to_vec(),
            b"old"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn source_of_move_remains_until_complete_copy_succeeds() {
        let src_root = unique_temp_dir("move-src-preserved");
        fs::create_dir_all(src_root.join("docs/nested")).unwrap();
        fs::write(src_root.join("docs/a.txt"), "a").unwrap();
        fs::write(src_root.join("docs/nested/b.txt"), "b").unwrap();
        let dst_root = unique_temp_dir("move-dst-blocked");
        let target = dst_root.join("target");
        fs::create_dir_all(&target).unwrap();

        let from_op = Operator::new(Fs::default().root(src_root.to_str().unwrap())).unwrap();
        let to_op = Operator::new(Fs::default().root(dst_root.to_str().unwrap())).unwrap();

        let lock_target = target.clone();
        let result = transfer_entries_with_progress(
            &from_op,
            &to_op,
            vec!["docs".to_string()],
            "target",
            TransferOperation::Move,
            false,
            TransferConflictPolicy::Fail,
            move |progress| {
                if progress.completed_items > 0 {
                    let _ = fs::set_permissions(&lock_target, fs::Permissions::from_mode(0o500));
                }
            },
            || false,
        )
        .await;

        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err());
        assert!(from_op.exists("docs/a.txt").await.unwrap());
        assert!(from_op.exists("docs/nested/b.txt").await.unwrap());
        assert!(!to_op.exists("target/docs/").await.unwrap());

        fs::remove_dir_all(&src_root).unwrap();
        fs::remove_dir_all(&dst_root).unwrap();
    }

    /// A minimal flat service that advertises optional `start_after` continuation.
    ///
    /// Entries are stored as a sorted map of relative paths. Listing emits keys
    /// strictly after `OpList::start_after`, mirroring S3/GCS-style services.
    struct FlatSimService {
        entries: std::collections::BTreeMap<String, u64>,
        info: opendal::raw::ServiceInfo,
        capability: opendal::Capability,
    }

    impl std::fmt::Debug for FlatSimService {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("FlatSimService")
                .finish_non_exhaustive()
        }
    }

    impl FlatSimService {
        fn new(
            supports_start_after: bool,
            entries: std::collections::BTreeMap<String, u64>,
        ) -> Self {
            let capability = opendal::Capability {
                list: true,
                list_with_recursive: true,
                list_with_start_after: supports_start_after,
                stat: true,
                ..Default::default()
            };
            let info = opendal::raw::ServiceInfo::new(
                "flat-sim",
                "/",
                format!("flat-sim-{}", entries.len()),
            );
            Self {
                entries,
                info,
                capability,
            }
        }
    }

    struct FlatSimLister {
        entries: std::vec::IntoIter<(String, u64)>,
        start_after: Option<String>,
    }

    impl opendal::raw::oio::List for FlatSimLister {
        async fn next(&mut self) -> opendal::Result<Option<opendal::raw::oio::Entry>> {
            loop {
                let Some((path, size)) = self.entries.next() else {
                    return Ok(None);
                };
                if let Some(after) = &self.start_after {
                    if path.as_str() <= after.as_str() {
                        continue;
                    }
                }
                return Ok(Some(opendal::raw::oio::Entry::new(
                    &path,
                    opendal::Metadata::new(opendal::EntryMode::FILE).with_content_length(size),
                )));
            }
        }
    }

    impl opendal::raw::Service for FlatSimService {
        type Reader = opendal::raw::oio::Reader;
        type Writer = opendal::raw::oio::Writer;
        type Lister = opendal::raw::oio::HierarchyLister<FlatSimLister>;
        type Deleter = ();
        type Copier = ();

        fn info(&self) -> opendal::raw::ServiceInfo {
            self.info.clone()
        }

        fn capability(&self) -> opendal::Capability {
            self.capability
        }

        async fn create_dir(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: opendal::raw::OpCreateDir,
        ) -> opendal::Result<opendal::raw::RpCreateDir> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        async fn stat(
            &self,
            _: &opendal::OperationContext,
            path: &str,
            _: opendal::raw::OpStat,
        ) -> opendal::Result<opendal::raw::RpStat> {
            let path = path.trim_end_matches('/');
            if path.is_empty() {
                return Ok(opendal::raw::RpStat::new(opendal::Metadata::new(
                    opendal::EntryMode::DIR,
                )));
            }
            match self.entries.get(path) {
                Some(size) => Ok(opendal::raw::RpStat::new(
                    opendal::Metadata::new(opendal::EntryMode::FILE).with_content_length(*size),
                )),
                None => Err(opendal::Error::new(
                    opendal::ErrorKind::NotFound,
                    "not found",
                )),
            }
        }

        fn read(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: opendal::raw::OpRead,
        ) -> opendal::Result<Self::Reader> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        fn write(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: opendal::raw::OpWrite,
        ) -> opendal::Result<Self::Writer> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        fn delete(&self, _: &opendal::OperationContext) -> opendal::Result<Self::Deleter> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        fn list(
            &self,
            _: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpList,
        ) -> opendal::Result<Self::Lister> {
            let entries = self.entries.clone().into_iter().collect::<Vec<_>>();
            Ok(opendal::raw::oio::HierarchyLister::new(
                FlatSimLister {
                    entries: entries.into_iter(),
                    start_after: args.start_after().map(str::to_string),
                },
                path,
                args.recursive(),
            ))
        }

        fn copy(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: &str,
            _: opendal::raw::OpCopy,
            _: opendal::raw::OpCopier,
        ) -> opendal::Result<Self::Copier> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        async fn rename(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: &str,
            _: opendal::raw::OpRename,
        ) -> opendal::Result<opendal::raw::RpRename> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }

        async fn presign(
            &self,
            _: &opendal::OperationContext,
            _: &str,
            _: opendal::raw::OpPresign,
        ) -> opendal::Result<opendal::raw::RpPresign> {
            Err(opendal::Error::new(
                opendal::ErrorKind::Unsupported,
                "unsupported",
            ))
        }
    }

    fn flat_sim_operator(
        supports_start_after: bool,
        entries: std::collections::BTreeMap<String, u64>,
    ) -> Operator {
        let service = Arc::new(FlatSimService::new(supports_start_after, entries));
        Operator::from_parts(
            opendal::OperationContext::default(),
            service as opendal::raw::Servicer,
        )
    }

    #[tokio::test]
    async fn non_recursive_simulator_reaches_entries_beyond_ten_thousand_with_start_after() {
        let entries = (0..10_001_u32)
            .map(|index| (format!("entry-{index:05}.txt"), 1u64))
            .collect::<std::collections::BTreeMap<_, _>>();
        let op = flat_sim_operator(true, entries);

        let mut cursor = None;
        let mut seen = 0usize;
        let mut saw_last = false;
        let mut truncated_any = false;
        loop {
            let page = list_entries_page(&op, "/", MAX_LIST_LIMIT, cursor, false, 1)
                .await
                .unwrap();
            seen += page.entries.len();
            truncated_any |= page.truncated;
            if page
                .entries
                .iter()
                .any(|entry| entry.path == "entry-10000.txt")
            {
                saw_last = true;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, 10_001, "entries after 10,000 must remain reachable");
        assert!(saw_last, "entry #10000 must be reachable via continuation");
        assert!(!truncated_any, "no scan cap should truncate the listing");
    }

    #[tokio::test]
    async fn non_recursive_replay_is_explicitly_truncated_at_documented_maximum() {
        let entries = (0..=MAX_PAGE_NON_RECURSIVE_SCANNED as u32)
            .map(|index| (format!("entry-{index:05}.txt"), 1u64))
            .collect::<std::collections::BTreeMap<_, _>>();
        let op = flat_sim_operator(false, entries);

        let mut cursor = None;
        let mut seen = 0usize;
        loop {
            let page = list_entries_page(&op, "/", MAX_LIST_LIMIT, cursor, false, 1)
                .await
                .unwrap();
            seen += page.entries.len();
            let next_cursor = page.next_cursor.clone();
            if next_cursor.is_none() {
                assert!(
                    page.truncated,
                    "stopping at the documented maximum must be explicit"
                );
                break;
            }
            cursor = next_cursor;
        }
        assert_eq!(
            seen, MAX_PAGE_NON_RECURSIVE_SCANNED,
            "replay-based non-recursive listing must stop at the documented maximum"
        );
    }
}
