//! Read-only, capability-neutral destination collision planning.
//!
//! Callers must first bind a trusted storage operator, validate destination
//! paths against backend and namespace policy, and replan under publication
//! locks at apply time. This module never grants publication authority, creates
//! storage objects, or overwrites destination data.
use std::collections::HashSet;

use infimount_core::CoreError;
use opendal::{ErrorKind, Operator};

use crate::publication::{AgentTaskPublishConflictPolicy, MAX_AGENT_TASK_DESTINATION_PATH_BYTES};
use crate::publication_plan::AgentTaskPublishAction;

/// Preserve the existing create/fail/rename preview policy. A conflict plan
/// must remain non-publishable; caller checks conflict counts before apply.
pub async fn choose_publication_destination(
    op: &Operator,
    base_path: &str,
    policy: &AgentTaskPublishConflictPolicy,
) -> Result<(String, AgentTaskPublishAction), CoreError> {
    match path_exists(op, base_path).await? {
        false => Ok((base_path.to_string(), AgentTaskPublishAction::Create)),
        true if *policy == AgentTaskPublishConflictPolicy::Fail => {
            Ok((base_path.to_string(), AgentTaskPublishAction::Conflict))
        }
        true => Ok((
            unique_destination_path(op, base_path).await?,
            AgentTaskPublishAction::Rename,
        )),
    }
}

/// Fail closed on case-folded duplicate destination paths within one plan.
/// The caller must also recheck the actual destination before create-only
/// publication; this reservation is not an external storage lock.
pub fn reserve_publication_destination_key(
    reserved: &mut HashSet<String>,
    destination_path: &str,
) -> Result<(), CoreError> {
    if !reserved.insert(destination_path.to_lowercase()) {
        return Err(CoreError::Config(
            "Selected Agent Task outputs collide at the publication destination".into(),
        ));
    }
    Ok(())
}

async fn unique_destination_path(op: &Operator, base_path: &str) -> Result<String, CoreError> {
    let (parent, name) = split_parent(base_path)?;
    let (stem, extension) = split_file_name(name);
    for index in 1..=9_999u32 {
        let suffix = if index == 1 {
            " copy".to_string()
        } else {
            format!(" copy {index}")
        };
        let candidate = join_path(&parent, &format!("{stem}{suffix}{extension}"));
        if candidate.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES {
            return Err(CoreError::Config(
                "renamed publication destination exceeds the path length limit".into(),
            ));
        }
        if !path_exists(op, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(CoreError::Config(
        "could not choose a non-conflicting publication destination".into(),
    ))
}

async fn path_exists(op: &Operator, path: &str) -> Result<bool, CoreError> {
    match op.stat(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim().trim_matches('/');
    let relative = relative.trim().trim_matches('/');
    match (root.is_empty(), relative.is_empty()) {
        (true, true) => String::new(),
        (true, false) => relative.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}/{relative}"),
    }
}

fn split_parent(path: &str) -> Result<(String, &str), CoreError> {
    let normalized = path.trim_matches('/');
    if normalized.is_empty() {
        return Err(CoreError::Config(
            "publication destination file path is empty".into(),
        ));
    }
    Ok(match normalized.rsplit_once('/') {
        Some((parent, name)) => (parent.to_string(), name),
        None => (String::new(), normalized),
    })
}

fn split_file_name(name: &str) -> (String, String) {
    if name.starts_with('.') {
        return (name.to_string(), String::new());
    }
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem.to_string(), format!(".{extension}"))
        }
        _ => (name.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_operator() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    #[tokio::test]
    async fn missing_destination_is_create_and_policy_does_not_change_it() {
        let op = memory_operator();
        for policy in [
            AgentTaskPublishConflictPolicy::Fail,
            AgentTaskPublishConflictPolicy::Rename,
        ] {
            let (path, action) = choose_publication_destination(&op, "exports/report.md", &policy)
                .await
                .unwrap();
            assert_eq!(path, "exports/report.md");
            assert_eq!(action, AgentTaskPublishAction::Create);
        }
        assert!(!path_exists(&op, "exports/report.md").await.unwrap());
    }

    #[tokio::test]
    async fn existing_destination_is_conflict_for_fail_policy() {
        let op = memory_operator();
        op.write("exports/report.md", "existing").await.unwrap();
        let (path, action) = choose_publication_destination(
            &op,
            "exports/report.md",
            &AgentTaskPublishConflictPolicy::Fail,
        )
        .await
        .unwrap();
        assert_eq!(path, "exports/report.md");
        assert_eq!(action, AgentTaskPublishAction::Conflict);
    }

    #[tokio::test]
    async fn rename_uses_legacy_copy_suffix_and_does_not_mutate_storage() {
        let op = memory_operator();
        op.write("exports/report.md", "original").await.unwrap();
        op.write("exports/report copy.md", "other").await.unwrap();
        let (path, action) = choose_publication_destination(
            &op,
            "exports/report.md",
            &AgentTaskPublishConflictPolicy::Rename,
        )
        .await
        .unwrap();
        assert_eq!(path, "exports/report copy 2.md");
        assert_eq!(action, AgentTaskPublishAction::Rename);
        assert!(!path_exists(&op, &path).await.unwrap());
        assert_eq!(
            op.read("exports/report.md").await.unwrap().to_vec(),
            b"original"
        );
    }

    #[tokio::test]
    async fn rename_keeps_dotfile_stem_without_extension() {
        let op = memory_operator();
        op.write("exports/.env", "existing").await.unwrap();
        let (path, action) = choose_publication_destination(
            &op,
            "exports/.env",
            &AgentTaskPublishConflictPolicy::Rename,
        )
        .await
        .unwrap();
        assert_eq!(path, "exports/.env copy");
        assert_eq!(action, AgentTaskPublishAction::Rename);
    }

    #[tokio::test]
    async fn rename_fails_on_path_limit_without_writing() {
        let op = memory_operator();
        let long_name = "x".repeat(MAX_AGENT_TASK_DESTINATION_PATH_BYTES);
        let error = unique_destination_path(&op, &long_name).await.unwrap_err();
        assert!(error.to_string().contains("path length limit"));
    }

    #[test]
    fn per_plan_case_folded_path_reservations_reject_collisions() {
        let mut seen = HashSet::new();
        reserve_publication_destination_key(&mut seen, "exports/Report.md").unwrap();
        assert!(reserve_publication_destination_key(&mut seen, "exports/report.md").is_err());
        reserve_publication_destination_key(&mut seen, "exports/another.md").unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn rejects_empty_file_destination_for_rename() {
        assert!(split_parent("").is_err());
        assert_eq!(split_file_name(".env"), (".env".into(), String::new()));
        assert_eq!(
            split_file_name("report.md"),
            ("report".into(), ".md".into())
        );
    }
}
