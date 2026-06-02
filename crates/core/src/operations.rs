use futures::io::{AsyncReadExt, AsyncWriteExt};
use futures::TryStreamExt;
use opendal::{ErrorKind, Operator};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

use crate::models::{Entry, Result};
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
        let name = extract_filename(&full_path);

        // Use op.stat on the full path to ensure we get full metadata.
        // If the entry no longer exists (e.g., broken symlink), keep the
        // entry but leave size/modified blank instead of failing or skipping.
        let (is_dir, size, modified_at) = match op.stat(&full_path).await {
            Ok(meta) => (
                meta.is_dir(),
                meta.content_length(),
                meta.last_modified().map(|dt| dt.to_string()),
            ),
            Err(e) if e.kind() == ErrorKind::NotFound => (false, 0, None),
            Err(e) => return Err(e.into()),
        };

        let entry = Entry {
            path: full_path,
            name,
            is_dir,
            size,
            modified_at,
        };

        out.push(entry);
    }

    Ok(out)
}

/// Recursively list entries below the given path using the provided operator.
pub async fn list_entries_recursive(op: &Operator, path: &str) -> Result<Vec<Entry>> {
    let root = normalize_list_path(path);
    let mut out = Vec::new();
    let mut stack = vec![root];

    while let Some(base) = stack.pop() {
        let mut lister = op.lister(&base).await?;
        while let Some(obj) = lister.try_next().await? {
            let full_path = obj.path().to_string();
            let name = extract_filename(&full_path);
            if full_path.is_empty() || name == "." {
                continue;
            }

            let meta = op.stat(&full_path).await?;
            if meta.is_dir() {
                stack.push(ensure_dir_path(&full_path));
            }

            out.push(Entry {
                path: full_path,
                name,
                is_dir: meta.is_dir(),
                size: if meta.is_dir() {
                    0
                } else {
                    meta.content_length()
                },
                modified_at: meta.last_modified().map(|dt| dt.to_string()),
            });
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
    })
}

/// Read the full contents of a file.
pub async fn read_full(op: &Operator, path: &str) -> Result<Vec<u8>> {
    let p = normalize_opendal_path(path);
    let data = op.read(&p).await?;
    Ok(data.to_vec())
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

async fn ensure_parent_dir(op: &Operator, path: &str) -> Result<()> {
    if let Some(parent) = parent_dir_path(path) {
        let parent_dir = ensure_dir_path(&parent);
        op.create_dir(&parent_dir).await?;
    }
    Ok(())
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
    let meta = from_op.stat(from).await?;
    let size = meta.content_length();
    let mut reader = from_op
        .reader(from)
        .await?
        .into_futures_async_read(0..size)
        .await?;
    let mut writer = to_op.writer(to).await?.into_futures_async_write();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        if let Some(callback) = is_cancelled {
            ensure_not_cancelled(callback)?;
        }
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await?;
        if let Some(callback) = on_bytes.as_mut() {
            callback(read as u64);
        }
        if let Some(callback) = is_cancelled {
            ensure_not_cancelled(callback)?;
        }
    }

    writer.close().await?;
    Ok(())
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
    let meta = from_op.stat(source_path).await?;
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
            let child_meta = from_op.stat(&child_path).await?;
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
    let meta = op.stat(path).await?;
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
            let child_meta = op.stat(&child_path).await?;
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
                from_op.copy(from_path, to_path).await?;
            } else {
                copy_file_across_operators(from_op, to_op, from_path, to_path).await?;
            }
        }
        TransferOperation::Move => {
            if same_source {
                from_op.rename(from_path, to_path).await?;
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
    let size = from_op.stat(from_path).await?.content_length();

    match operation {
        TransferOperation::Copy => {
            if same_source {
                from_op.copy(from_path, to_path).await?;
                state.bytes_transferred = state.bytes_transferred.saturating_add(size);
                emit_progress(progress, state, from_path);
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
                from_op.rename(from_path, to_path).await?;
                state.bytes_transferred = state.bytes_transferred.saturating_add(size);
                emit_progress(progress, state, from_path);
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
    let from_root = ensure_dir_path(from_dir);
    let to_root = ensure_dir_path(to_dir);
    to_op.create_dir(&to_root).await?;

    let mut stack = vec![(from_root.clone(), to_root)];
    while let Some((from_base, to_base)) = stack.pop() {
        let mut lister = from_op.lister(&from_base).await?;
        while let Some(obj) = lister.try_next().await? {
            let child_path = obj.path().to_string();
            let meta = from_op.stat(&child_path).await?;
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
            let meta = from_op.stat(&child_path).await?;
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
    let mut entries = Vec::new();

    for source_path in &paths {
        let meta = from_op.stat(source_path).await?;
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
    if conflict_policy == TransferConflictPolicy::Fail {
        for from_path in &paths {
            let meta = from_op.stat(from_path).await?;

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
        let meta = from_op.stat(&from_path).await?;
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
    let (total_items, total_bytes) = estimate_transfer_totals(from_op, &paths).await?;
    let mut state = TransferProgressState {
        completed_items: 0,
        total_items,
        bytes_transferred: 0,
        total_bytes,
    };
    emit_progress(&mut progress, &state, "");

    if conflict_policy == TransferConflictPolicy::Fail {
        for from_path in &paths {
            ensure_not_cancelled(&is_cancelled)?;
            let meta = from_op.stat(from_path).await?;

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
        let meta = from_op.stat(&from_path).await?;
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

        let data = fs::read(src).await.map_err(|e| {
            opendal::Error::new(
                ErrorKind::Unexpected,
                format!("Failed to read local file {}: {}", src.display(), e),
            )
        })?;

        op.write(&target_path, data).await?;
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
                    let data = fs::read(&child_path).await.map_err(|e| {
                        opendal::Error::new(
                            ErrorKind::Unexpected,
                            format!("Failed to read local file {}: {}", child_path.display(), e),
                        )
                    })?;
                    op.write(&target_path, data).await?;
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

use serde::Deserialize;

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
    use opendal::services::Memory;

    async fn create_test_operator() -> Operator {
        let builder = Memory::default();
        Operator::new(builder).unwrap().finish()
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
