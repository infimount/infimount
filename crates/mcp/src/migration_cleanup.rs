use crate::errors::{err, McpErrorCode, McpResult};
use infimount_core::atomic_file::atomic_write_file;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

const JOURNAL_FILE: &str = "plaintext-cleanup.json";
const MAX_PENDING_FILES: usize = 128;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupJournal {
    #[serde(default)]
    pending_files: Vec<String>,
}

fn journal_path(backups_dir: &Path) -> PathBuf {
    backups_dir.join(JOURNAL_FILE)
}

fn safe_file_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if name.is_empty()
        || name == JOURNAL_FILE
        || Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(name.to_string())
}

fn read_journal(backups_dir: &Path) -> CleanupJournal {
    fs::read(journal_path(backups_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_journal(backups_dir: &Path, journal: &CleanupJournal) -> McpResult<()> {
    let path = journal_path(backups_dir);
    if journal.pending_files.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "plaintext migration cleanup remains pending",
                ))
            }
        }
    }
    let payload = serde_json::to_vec(journal).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "failed to record plaintext migration cleanup",
        )
    })?;
    atomic_write_file(&path, &payload, 0o600).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "failed to record plaintext migration cleanup",
        )
    })
}

/// Delete a plaintext migration backup and verify it is gone. If deletion cannot
/// complete, persist only its bounded filename (never file contents or secrets)
/// so startup can retry safely.
pub fn delete_plaintext_backup_or_journal(backup_path: &Path) -> McpResult<()> {
    let backups_dir = backup_path.parent().ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "plaintext migration cleanup path is invalid",
        )
    })?;
    let file_name = safe_file_name(backup_path).ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "plaintext migration cleanup path is invalid",
        )
    })?;

    let removed = match fs::remove_file(backup_path) {
        Ok(()) => !backup_path.exists(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    if removed {
        return Ok(());
    }

    let mut journal = read_journal(backups_dir);
    if !journal.pending_files.iter().any(|item| item == &file_name) {
        if journal.pending_files.len() >= MAX_PENDING_FILES {
            journal.pending_files.remove(0);
        }
        journal.pending_files.push(file_name);
    }
    write_journal(backups_dir, &journal)?;
    Err(err(
        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
        "plaintext migration cleanup remains pending",
    ))
}

/// Retry pending cleanup. Returns the number of files that still could not be
/// removed. Invalid or corrupt journals fail closed without exposing paths.
pub fn retry_pending_plaintext_cleanup(config_dir: &Path) -> McpResult<usize> {
    let backups_dir = config_dir.join("backups");
    let path = journal_path(&backups_dir);
    if !path.exists() {
        return Ok(0);
    }
    let bytes = fs::read(&path).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "failed to read plaintext migration cleanup state",
        )
    })?;
    let mut journal: CleanupJournal = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "plaintext migration cleanup state is invalid",
        )
    })?;
    if journal.pending_files.len() > MAX_PENDING_FILES
        || journal
            .pending_files
            .iter()
            .any(|name| safe_file_name(Path::new(name)).as_deref() != Some(name.as_str()))
    {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "plaintext migration cleanup state is invalid",
        ));
    }

    journal.pending_files.retain(|name| {
        let candidate = backups_dir.join(name);
        match fs::remove_file(&candidate) {
            Ok(()) => candidate.exists(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        }
    });
    let remaining = journal.pending_files.len();
    write_journal(&backups_dir, &journal)?;
    if remaining > 0 {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "plaintext migration cleanup remains pending",
        ));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_plaintext_backup_and_leaves_no_journal() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("backups");
        fs::create_dir_all(&backups).unwrap();
        let backup = backups.join("storages.pre.json");
        fs::write(&backup, b"sensitive").unwrap();

        delete_plaintext_backup_or_journal(&backup).unwrap();
        assert!(!backup.exists());
        assert!(!journal_path(&backups).exists());
    }

    #[test]
    fn failed_deletion_records_only_bounded_filename_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("backups");
        fs::create_dir_all(&backups).unwrap();
        let undeletable_as_file = backups.join("storages.pre.json");
        fs::create_dir(&undeletable_as_file).unwrap();

        let error = delete_plaintext_backup_or_journal(&undeletable_as_file).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_MIGRATION_FAILED);
        let raw = fs::read_to_string(journal_path(&backups)).unwrap();
        assert!(raw.contains("storages.pre.json"));
        assert!(!raw.contains(dir.path().to_string_lossy().as_ref()));

        fs::remove_dir(&undeletable_as_file).unwrap();
        retry_pending_plaintext_cleanup(dir.path()).unwrap();
        assert!(!journal_path(&backups).exists());
    }
}
