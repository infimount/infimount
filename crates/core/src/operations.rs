use futures::TryStreamExt;
use opendal::{ErrorKind, Operator};

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
    op.delete(path).await?;
    Ok(())
}
