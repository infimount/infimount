use futures::TryStreamExt;
use futures::io::AsyncWriteExt;
use opendal::{ErrorKind, Operator};
use std::path::Path;
use tokio::fs;

use crate::models::{Entry, Result};
use crate::util::extract_filename;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOperation {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferConflictPolicy {
    /// Fail fast if any destination exists (no partial transfer).
    Fail,
    /// Replace destination objects when they already exist.
    Overwrite,
    /// Skip entries whose destinations already exist.
    Skip,
}

/// List entries at the given path using the provided operator.
pub async fn list_entries(op: &Operator, path: &str) -> Result<Vec<Entry>> {
    let mut lister = op.lister(path).await?;
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
                meta.last_modified().map(|dt| dt.to_rfc3339()),
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

/// Stat a single entry.
pub async fn stat_entry(op: &Operator, path: &str) -> Result<Entry> {
    let meta = op.stat(path).await?;
    let full_path = path.to_string();
    let name = extract_filename(&full_path);

    Ok(Entry {
        path: full_path,
        name,
        is_dir: meta.is_dir(),
        size: meta.content_length(),
        modified_at: meta
            .last_modified()
            .map(|dt| dt.to_rfc3339()),
    })
}

/// Read the full contents of a file.
pub async fn read_full(op: &Operator, path: &str) -> Result<Vec<u8>> {
    let data = op.read(path).await?;
    Ok(data.to_vec())
}

/// Write the full contents of a file, overwriting if it exists.
pub async fn write_full(op: &Operator, path: &str, data: &[u8]) -> Result<()> {
    op.write(path, data.to_vec()).await?;
    Ok(())
}

/// Delete a path (file or directory).
pub async fn delete(op: &Operator, path: &str) -> Result<()> {
    op.remove_all(path).await?;
    Ok(())
}

/// Upload files from local paths to the target directory.
pub async fn upload_files_from_paths(op: &Operator, paths: Vec<String>, target_dir: String) -> Result<()> {
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

async fn copy_file_across_operators(from_op: &Operator, to_op: &Operator, from: &str, to: &str) -> Result<()> {
    let meta = from_op.stat(from).await?;
    let size = meta.content_length();
    let mut reader = from_op
        .reader(from)
        .await?
        .into_futures_async_read(0..size)
        .await?;
    let mut writer = to_op.writer(to).await?.into_futures_async_write();

    futures::io::copy(&mut reader, &mut writer).await?;
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
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem.to_string(), format!(".{}", ext)),
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

    if !op.exists(&candidate).await? {
        return Ok(candidate);
    }

    if is_dir {
        let base_name = name.to_string();
        for idx in 1..=9999u32 {
            let suffix = if idx == 1 { " copy".to_string() } else { format!(" copy {}", idx) };
            let next_name = format!("{}{}", base_name, suffix);
            candidate = ensure_dir_path(&join_target_dir(target_dir, &next_name));
            if !op.exists(&candidate).await? {
                return Ok(candidate);
            }
        }
    } else {
        let (stem, ext) = split_file_name(name);
        for idx in 1..=9999u32 {
            let suffix = if idx == 1 { " copy".to_string() } else { format!(" copy {}", idx) };
            let next_name = format!("{}{}{}", stem, suffix, ext);
            candidate = join_target_dir(target_dir, &next_name);
            if !op.exists(&candidate).await? {
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
                from_op.remove_all(from_path).await?;
            }
        }
    }

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
        from_op.remove_all(&from_root).await?;
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
                    if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src {
                        return Err(opendal::Error::new(
                            ErrorKind::IsSameFile,
                            "Cannot copy a folder into itself",
                        )
                        .into());
                    }
                }

                // Copying onto itself is treated as "duplicate" (keep both) and won't conflict.
                if operation == TransferOperation::Copy && same_source && normalized_src == normalized_dest {
                    continue;
                }

                if to_op.exists(&dest_dir).await? {
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

                if to_op.exists(&dest_file).await? {
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
                if normalized_dest.starts_with(&normalized_src) && normalized_dest != normalized_src {
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

            if to_op.exists(&dest_dir).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination directory already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        to_op.remove_all(&dest_dir).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                }
            }

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
            let dest_file =
                if operation == TransferOperation::Copy && same_source && from_path == base_dest_file {
                    unique_destination_path(to_op, target_dir, &file_name, false).await?
                } else {
                    base_dest_file
                };

            if operation == TransferOperation::Move && same_source && from_path == dest_file {
                continue;
            }

            if to_op.exists(&dest_file).await? {
                match conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(opendal::Error::new(
                            ErrorKind::AlreadyExists,
                            "Destination file already exists",
                        )
                        .into())
                    }
                    TransferConflictPolicy::Overwrite => {
                        to_op.remove_all(&dest_file).await?;
                    }
                    TransferConflictPolicy::Skip => {
                        continue;
                    }
                }
            }
            transfer_file(from_op, to_op, &from_path, &dest_file, operation, same_source).await?;
        }
    }

    Ok(())
}

async fn upload_path_recursive(op: &Operator, src: &Path, target_dir: &str) -> Result<()> {
    let meta = fs::metadata(src).await.map_err(|e| {
        opendal::Error::new(
            ErrorKind::Unexpected,
            &format!("Failed to stat local path {}: {}", src.display(), e),
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
                &format!("Failed to read local file {}: {}", src.display(), e),
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
                    &format!("Failed to read directory {}: {}", dir_path.display(), e),
                )
            })?;

            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                opendal::Error::new(
                    ErrorKind::Unexpected,
                    &format!(
                        "Failed to iterate directory {}: {}",
                        dir_path.display(),
                        e
                    ),
                )
            })? {
                let child_path = entry.path();
                let child_meta = fs::metadata(&child_path).await.map_err(|e| {
                    opendal::Error::new(
                        ErrorKind::Unexpected,
                        &format!(
                            "Failed to stat local path {}: {}",
                            child_path.display(),
                            e
                        ),
                    )
                })?;

                if child_meta.is_file() {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    let target_path = join_target_dir(&dir_target, &filename);
                    let data = fs::read(&child_path).await.map_err(|e| {
                        opendal::Error::new(
                            ErrorKind::Unexpected,
                            &format!(
                                "Failed to read local file {}: {}",
                                child_path.display(),
                                e
                            ),
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
        op.write("dir1/file2.txt", "content2".as_bytes()).await.unwrap();

        let entries = list_entries(&op, "/").await.unwrap();
        assert_eq!(entries.len(), 2); // file1.txt and dir1

        let file1 = entries.iter().find(|e| e.name == "file1.txt").unwrap();
        assert!(!file1.is_dir);
        assert_eq!(file1.size, 8);

        let dir1 = entries.iter().find(|e| e.name == "dir1").unwrap();
        assert!(dir1.is_dir);
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
    async fn test_delete_file() {
        let op = create_test_operator().await;
        let path = "todelete.txt";
        op.write(path, "bye".as_bytes()).await.unwrap();

        delete(&op, path).await.unwrap();

        let exists = op.exists(path).await.unwrap();
        assert!(!exists);
    }
}
