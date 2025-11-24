use futures::TryStreamExt;
use opendal::{ErrorKind, Operator};
use std::path::Path;
use tokio::fs;

use crate::models::{Entry, Result};
use crate::util::extract_filename;

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
