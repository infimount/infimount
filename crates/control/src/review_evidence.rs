//! Read-only, bounded Agent Task output listing shared by trusted review clients.
//!
//! The caller must bind the workspace and task, validate its output directory,
//! and validate every returned path against local storage confinement before
//! reading, hashing, previewing or publishing. This module grants no authority.
use infimount_core::models::{Entry, MAX_RECURSIVE_ITEMS};
use infimount_core::{operations, CoreError};
use opendal::Operator;

pub const MAX_AGENT_TASK_OUTPUT_REVIEW_FILES: usize = 1_000;
pub const MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Preserve the desktop's fail-closed behavior at the recursive listing cap
/// and return only sorted file entries with bounded listed byte size.
pub async fn list_review_output_entries(
    op: &Operator,
    confined_outputs_path: &str,
) -> Result<Vec<Entry>, CoreError> {
    let entries = operations::list_entries_recursive(op, confined_outputs_path)
        .await
        .map_err(|_| CoreError::Config("Agent Task outputs could not be enumerated".into()))?;
    validate_review_listing(entries)
}

fn validate_review_listing(entries: Vec<Entry>) -> Result<Vec<Entry>, CoreError> {
    // The common recursive lister returns an apparently successful but
    // truncated result at this cap; review must reject incomplete evidence.
    if entries.len() >= MAX_RECURSIVE_ITEMS as usize {
        return Err(CoreError::Config(
            "Agent Task outputs are too large to review without truncation".into(),
        ));
    }

    let mut files = entries
        .into_iter()
        .filter(|entry| !entry.is_dir)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if files.len() > MAX_AGENT_TASK_OUTPUT_REVIEW_FILES {
        return Err(CoreError::Config(format!(
            "Agent Task output review supports at most {MAX_AGENT_TASK_OUTPUT_REVIEW_FILES} files"
        )));
    }
    let listed_bytes = files.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.size)
            .ok_or_else(|| CoreError::Config("Agent Task output byte count overflowed".into()))
    })?;
    if listed_bytes > MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES {
        return Err(CoreError::Config(
            "Agent Task outputs exceed the 2 GiB review limit".into(),
        ));
    }
    Ok(files)
}

/// Accumulate the actual hashed byte counts, which can differ from the
/// listing metadata if an external writer changes a file during review.
pub fn checked_reviewed_output_bytes(total: u64, hashed_size: u64) -> Result<u64, CoreError> {
    let next = total
        .checked_add(hashed_size)
        .ok_or_else(|| CoreError::Config("Agent Task output byte count overflowed".into()))?;
    if next > MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES {
        return Err(CoreError::Config(
            "Agent Task outputs changed beyond the 2 GiB review limit".into(),
        ));
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> Entry {
        Entry {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_dir: false,
            size,
            modified_at: None,
            etag: None,
        }
    }

    #[tokio::test]
    async fn recursively_lists_only_files_in_deterministic_path_order() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        op.create_dir("tasks/task/outputs/nested/").await.unwrap();
        op.write("tasks/task/outputs/z.txt", "last").await.unwrap();
        op.write("tasks/task/outputs/nested/a.txt", "first")
            .await
            .unwrap();

        let files = list_review_output_entries(&op, "tasks/task/outputs/")
            .await
            .unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "tasks/task/outputs/nested/a.txt",
                "tasks/task/outputs/z.txt"
            ]
        );
        assert_eq!(files.iter().map(|file| file.size).sum::<u64>(), 9);
    }

    #[test]
    fn refuses_truncated_recursive_listing_even_when_it_contains_only_directories() {
        let mut directory = file("outputs/subdir/", 0);
        directory.is_dir = true;
        let entries = vec![directory; MAX_RECURSIVE_ITEMS as usize];
        let error = validate_review_listing(entries).unwrap_err();
        assert!(error.to_string().contains("without truncation"));
    }

    #[test]
    fn rejects_oversized_file_count_but_does_not_count_directories_as_files() {
        let entries = (0..=MAX_AGENT_TASK_OUTPUT_REVIEW_FILES)
            .map(|n| file(&format!("outputs/{n}.txt"), 1))
            .collect();
        let error = validate_review_listing(entries).unwrap_err();
        assert!(error.to_string().contains("at most 1000 files"));

        let mut directory = file("outputs/empty/", 0);
        directory.is_dir = true;
        let entries = vec![
            file("outputs/z.txt", 1),
            directory,
            file("outputs/a.txt", 2),
        ];
        let files = validate_review_listing(entries).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "outputs/a.txt");
    }

    #[test]
    fn listed_metadata_rejects_excess_size_and_arithmetic_overflow() {
        let too_large = vec![file(
            "outputs/huge.bin",
            MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES + 1,
        )];
        assert!(validate_review_listing(too_large)
            .unwrap_err()
            .to_string()
            .contains("2 GiB review limit"));

        let overflowing = vec![file("outputs/a", u64::MAX), file("outputs/b", 1)];
        assert!(validate_review_listing(overflowing)
            .unwrap_err()
            .to_string()
            .contains("overflowed"));
    }

    #[test]
    fn rehashed_bytes_enforce_limit_even_if_listing_was_small() {
        assert_eq!(checked_reviewed_output_bytes(3, 4).unwrap(), 7);
        assert!(
            checked_reviewed_output_bytes(MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES, 1)
                .unwrap_err()
                .to_string()
                .contains("changed beyond")
        );
        assert!(checked_reviewed_output_bytes(u64::MAX, 1)
            .unwrap_err()
            .to_string()
            .contains("overflowed"));
    }
}
