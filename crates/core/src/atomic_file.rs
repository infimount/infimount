use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::models::{CoreError, Result};

pub const FILE_MODE: u32 = 0o600;
pub const DIR_MODE: u32 = 0o700;

pub fn atomic_write_file(path: &Path, payload: &[u8], mode: u32) -> Result<()> {
    ensure_parent(path)?;
    let parent = path.parent().ok_or_else(|| {
        CoreError::Config(format!("path has no parent directory: {}", path.display()))
    })?;

    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let tmp_path = parent.join(tmp_name);

    let write_result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options.open(&tmp_path)?;
        file.write_all(payload)?;
        file.sync_all()?;

        #[cfg(windows)]
        {
            replace_file_windows(&tmp_path, path)?;
        }
        #[cfg(not(windows))]
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(CoreError::Io(error));
    }

    #[cfg(unix)]
    {
        // The rename has committed at this point. A directory-sync failure must not
        // be reported as an uncommitted write, because callers would roll back the
        // newly referenced secret bundle and corrupt the committed registry.
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn create_dir_all(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or_else(|| {
            CoreError::Config("directory path has no existing ancestor".to_string())
        })?;
    }
    for directory in missing.into_iter().rev() {
        #[cfg(unix)]
        let result = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(DIR_MODE).create(&directory)
        };
        #[cfg(not(unix))]
        let result = fs::create_dir(&directory);

        if let Err(error) = result {
            if error.kind() != std::io::ErrorKind::AlreadyExists || !directory.is_dir() {
                return Err(CoreError::Io(error));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file_windows(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_file_with_content() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.json");
        let payload = b"hello world";

        atomic_write_file(&path, payload, FILE_MODE).expect("atomic write");
        assert!(path.exists());
        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn atomic_write_overwrites_safely() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.json");
        atomic_write_file(&path, b"first", FILE_MODE).expect("first write");
        atomic_write_file(&path, b"second", FILE_MODE).expect("second write");
        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "second");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_sets_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.json");
        atomic_write_file(&path, b"data", FILE_MODE).expect("atomic write");

        let metadata = fs::metadata(&path).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, FILE_MODE);
    }

    #[test]
    fn ensure_parent_sets_dir_permissions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("nested").join("deeper");
        ensure_parent(&nested.join("file.json")).expect("ensure parent");
        assert!(nested.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&nested).expect("metadata");
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, DIR_MODE);
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_parent_does_not_chmod_existing_ancestor() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("set mode");
        ensure_parent(&dir.path().join("new").join("file.json")).expect("ensure parent");
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(dir.path().join("new"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            DIR_MODE
        );
    }

    #[test]
    fn atomic_write_uses_atomic_temp_file_pattern() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("target.json");
        atomic_write_file(&path, b"data", FILE_MODE).expect("atomic write");

        let tmp_files: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(tmp_files.is_empty(), "temp files should be cleaned up");
    }

    #[test]
    fn atomic_write_fails_gracefully_on_bad_path() {
        let result = atomic_write_file(
            Path::new("/nonexistent/deep/path/file.json"),
            b"data",
            FILE_MODE,
        );
        assert!(result.is_err());
    }
}
