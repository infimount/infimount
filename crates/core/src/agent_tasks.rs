use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{CoreError, Result};

pub const AGENT_TASK_SCHEMA_VERSION: u32 = 1;
pub const AGENT_TASKS_DIR: &str = "tasks";
pub const AGENT_TASK_INPUTS_DIR: &str = "inputs";
pub const AGENT_TASK_OUTPUTS_DIR: &str = "outputs";
pub const AGENT_TASK_MANIFEST_FILE: &str = "task-manifest.json";
pub const AGENT_TASK_BRIEF_FILE: &str = "TASK.md";
pub const MAX_AGENT_TASK_INPUTS: usize = 10_000;
pub const MAX_AGENT_TASK_SELECTIONS: usize = 1_000;
pub const MAX_AGENT_TASK_PREPARED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_AGENT_TASK_REQUESTED_OUTPUTS: usize = 100;
pub const MAX_AGENT_TASK_TITLE_LEN: usize = 120;
pub const MAX_AGENT_TASK_OBJECTIVE_LEN: usize = 32 * 1024;
pub const MAX_AGENT_TASK_PATH_LEN: usize = 1_024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskInput {
    /// Relative path inside the task package, always under `inputs/`.
    /// Source storage identity/path is deliberately not persisted in this
    /// agent-readable manifest; the prepared bytes are the task input.
    pub task_path: String,
    pub byte_size: u64,
    /// Lowercase SHA-256 of the bytes prepared into the task package.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskManifest {
    pub schema_version: u32,
    pub task_id: String,
    pub title: String,
    pub created_at: String,
    /// Existing Agent Workspace that contains this task package.
    pub workspace_id: String,
    /// Workspace-relative direct child of `tasks/`, for example
    /// `tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454`.
    pub task_root: String,
    /// Kept explicit for forward-compatible readers; v1 requires `outputs`.
    pub outputs_directory: String,
    pub inputs: Vec<AgentTaskInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskBrief {
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub requested_outputs: Vec<String>,
}

pub fn agent_task_root(task_id: &str) -> Result<String> {
    validate_uuid("task id", task_id)?;
    Ok(format!("{AGENT_TASKS_DIR}/{task_id}"))
}

pub fn validate_agent_task_manifest(manifest: &AgentTaskManifest) -> Result<()> {
    if manifest.schema_version != AGENT_TASK_SCHEMA_VERSION {
        return config_error("unsupported Agent Task manifest schema");
    }
    validate_uuid("task id", &manifest.task_id)?;
    validate_uuid("workspace id", &manifest.workspace_id)?;
    validate_title(&manifest.title)?;
    chrono::DateTime::parse_from_rfc3339(&manifest.created_at)
        .map_err(|_| CoreError::Config("Agent Task createdAt must be RFC3339".to_string()))?;
    validate_task_root(&manifest.task_root)?;
    if manifest.task_root != agent_task_root(&manifest.task_id)? {
        return config_error("Agent Task root must be derived from its task id");
    }
    if manifest.outputs_directory != AGENT_TASK_OUTPUTS_DIR {
        return config_error("Agent Task outputsDirectory must be 'outputs'");
    }
    if manifest.inputs.len() > MAX_AGENT_TASK_INPUTS {
        return config_error(format!(
            "Agent Task may contain at most {MAX_AGENT_TASK_INPUTS} prepared inputs"
        ));
    }

    let mut seen_task_paths = HashSet::with_capacity(manifest.inputs.len());
    let mut seen_portable_paths = HashSet::with_capacity(manifest.inputs.len());
    for input in &manifest.inputs {
        validate_relative_child_path(&input.task_path, AGENT_TASK_INPUTS_DIR)?;
        validate_sha256(&input.sha256)?;
        if !seen_task_paths.insert(input.task_path.as_str()) {
            return config_error("Agent Task contains duplicate prepared input paths");
        }
        if !seen_portable_paths.insert(input.task_path.to_lowercase()) {
            return config_error(
                "Agent Task contains prepared input paths that collide case-insensitively",
            );
        }
    }
    Ok(())
}

pub fn validate_agent_task_brief(brief: &AgentTaskBrief) -> Result<()> {
    validate_title(&brief.title)?;
    if brief.objective.trim().is_empty() {
        return config_error("Agent Task objective must not be empty");
    }
    if brief.objective.len() > MAX_AGENT_TASK_OBJECTIVE_LEN {
        return config_error(format!(
            "Agent Task objective must be at most {MAX_AGENT_TASK_OBJECTIVE_LEN} bytes"
        ));
    }
    if brief.objective.chars().any(|ch| ch == '\0') {
        return config_error("Agent Task objective contains a NUL character");
    }
    if brief.requested_outputs.len() > MAX_AGENT_TASK_REQUESTED_OUTPUTS {
        return config_error(format!(
            "Agent Task may request at most {MAX_AGENT_TASK_REQUESTED_OUTPUTS} outputs"
        ));
    }

    let mut seen = HashSet::with_capacity(brief.requested_outputs.len());
    let mut seen_portable = HashSet::with_capacity(brief.requested_outputs.len());
    for output in &brief.requested_outputs {
        validate_relative_child_path(output, AGENT_TASK_OUTPUTS_DIR)?;
        if !seen.insert(output.as_str()) {
            return config_error("Agent Task contains duplicate requested output paths");
        }
        if !seen_portable.insert(output.to_lowercase()) {
            return config_error(
                "Agent Task contains requested output paths that collide case-insensitively",
            );
        }
    }
    Ok(())
}

/// Render the human- and agent-readable task brief written beside the manifest.
///
/// Security note: this document is task guidance, not an authorization source.
/// MCP workspace policy remains authoritative even if an agent modifies TASK.md.
pub fn render_agent_task_markdown(
    manifest: &AgentTaskManifest,
    brief: &AgentTaskBrief,
) -> Result<String> {
    validate_agent_task_manifest(manifest)?;
    validate_agent_task_brief(brief)?;
    if manifest.title != brief.title {
        return config_error("Agent Task manifest title and brief title must match");
    }

    let mut out = String::new();
    out.push_str("# ");
    out.push_str(brief.title.trim());
    out.push_str("\n\n## Objective\n\n");
    out.push_str(brief.objective.trim());
    out.push_str("\n\n## Prepared inputs\n\n");

    if manifest.inputs.is_empty() {
        out.push_str("- No prepared input files.\n");
    } else {
        for input in &manifest.inputs {
            out.push_str("- `");
            out.push_str(&input.task_path);
            out.push_str("`\n");
        }
    }

    out.push_str("\n## Outputs\n\n");
    if brief.requested_outputs.is_empty() {
        out.push_str("Place deliverables under `outputs/` for user review.\n");
    } else {
        out.push_str("Create these deliverables under `outputs/`:\n\n");
        for path in &brief.requested_outputs {
            out.push_str("- `");
            out.push_str(path);
            out.push_str("`\n");
        }
    }

    out.push_str(
        "\n## Boundary\n\nOnly files available inside this Agent Task workspace should be treated as task inputs. Publication is performed separately by the Infimount desktop after user review.\n",
    );
    Ok(out)
}

fn validate_title(title: &str) -> Result<()> {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_AGENT_TASK_TITLE_LEN {
        return config_error(format!(
            "Agent Task title must be between 1 and {MAX_AGENT_TASK_TITLE_LEN} bytes"
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return config_error("Agent Task title contains control characters");
    }
    Ok(())
}

fn validate_uuid(label: &str, value: &str) -> Result<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| CoreError::Config(format!("Agent Task {label} must be a UUID")))
}

fn validate_task_root(path: &str) -> Result<()> {
    validate_relative_path(path)?;
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() != 2 || segments[0] != AGENT_TASKS_DIR || segments[1].is_empty() {
        return config_error("Agent Task root must be exactly one direct child of 'tasks/'");
    }
    Ok(())
}

fn validate_relative_child_path(path: &str, parent: &str) -> Result<()> {
    validate_relative_path(path)?;
    let prefix = format!("{parent}/");
    if !path.starts_with(&prefix) || path.len() == prefix.len() {
        return config_error(format!("Agent Task path must be a child of '{parent}/'"));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > MAX_AGENT_TASK_PATH_LEN {
        return config_error(format!(
            "Agent Task path must be between 1 and {MAX_AGENT_TASK_PATH_LEN} bytes"
        ));
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return config_error("Agent Task paths must use relative forward-slash form");
    }
    if path.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return config_error("Agent Task path contains control characters");
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return config_error("Agent Task path contains an invalid segment");
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, 'a'..='f'))
    {
        return config_error("Agent Task input sha256 must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn config_error<T>(message: impl Into<String>) -> Result<T> {
    Err(CoreError::Config(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: &str) -> AgentTaskInput {
        AgentTaskInput {
            task_path: path.to_string(),
            byte_size: 42,
            sha256: "a".repeat(64),
        }
    }

    fn manifest() -> AgentTaskManifest {
        let task_id = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: task_id.to_string(),
            title: "Validate customer export".to_string(),
            created_at: "2026-09-07T12:00:00Z".to_string(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".to_string(),
            task_root: agent_task_root(task_id).unwrap(),
            outputs_directory: AGENT_TASK_OUTPUTS_DIR.to_string(),
            inputs: vec![input("inputs/customers.csv")],
        }
    }

    #[test]
    fn validates_current_manifest() {
        validate_agent_task_manifest(&manifest()).unwrap();
    }

    #[test]
    fn manifest_round_trips_without_unknown_authority_fields() {
        let manifest = manifest();
        let encoded = serde_json::to_string(&manifest).unwrap();
        let decoded: AgentTaskManifest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, manifest);
        assert!(!encoded.contains("sourcePath"));
        assert!(!encoded.contains("sourceStorageId"));
    }

    #[test]
    fn rejects_nested_or_mismatched_task_root() {
        let mut manifest = manifest();
        manifest.task_root = "tasks/one/nested".to_string();
        assert!(validate_agent_task_manifest(&manifest).is_err());

        manifest.task_root = "tasks/00000000-0000-0000-0000-000000000000".to_string();
        assert!(validate_agent_task_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_task_path_escape_and_backslashes() {
        let mut manifest = manifest();
        manifest.inputs[0].task_path = "inputs/../private.txt".to_string();
        assert!(validate_agent_task_manifest(&manifest).is_err());

        manifest.inputs[0].task_path = "inputs\\private.txt".to_string();
        assert!(validate_agent_task_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_duplicate_and_case_colliding_prepared_paths() {
        let mut duplicate = manifest();
        duplicate.inputs.push(input("inputs/customers.csv"));
        assert!(validate_agent_task_manifest(&duplicate).is_err());

        let mut case_collision = manifest();
        case_collision.inputs.push(input("inputs/CUSTOMERS.csv"));
        assert!(validate_agent_task_manifest(&case_collision).is_err());
    }

    #[test]
    fn rejects_noncanonical_sha256() {
        let mut manifest = manifest();
        manifest.inputs[0].sha256 = "A".repeat(64);
        assert!(validate_agent_task_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_requested_output_outside_outputs_directory() {
        let brief = AgentTaskBrief {
            title: manifest().title,
            objective: "Find invalid rows.".to_string(),
            requested_outputs: vec!["../report.md".to_string()],
        };
        assert!(validate_agent_task_brief(&brief).is_err());
    }

    #[test]
    fn rendered_brief_names_only_prepared_paths() {
        let manifest = manifest();
        let brief = AgentTaskBrief {
            title: manifest.title.clone(),
            objective: "Identify malformed records and summarize the findings.".to_string(),
            requested_outputs: vec![
                "outputs/invalid-records.csv".to_string(),
                "outputs/summary.md".to_string(),
            ],
        };
        let markdown = render_agent_task_markdown(&manifest, &brief).unwrap();
        assert!(markdown.contains("`inputs/customers.csv`"));
        assert!(markdown.contains("`outputs/summary.md`"));
        assert!(markdown.contains("Publication is performed separately by the Infimount desktop"));
    }

    #[test]
    fn serde_rejects_unknown_manifest_and_source_authority_fields() {
        let value = serde_json::json!({
            "schemaVersion": 1,
            "taskId": "81f08176-86e4-40ec-a9a4-a219c4c9b454",
            "title": "Task",
            "createdAt": "2026-09-07T12:00:00Z",
            "workspaceId": "f8f47aa7-702d-4fd6-8815-84cbdf3b3127",
            "taskRoot": "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454",
            "outputsDirectory": "outputs",
            "inputs": [{
                "taskPath": "inputs/a.txt",
                "byteSize": 1,
                "sha256": "a".repeat(64),
                "sourcePath": "private/root/a.txt"
            }]
        });
        assert!(serde_json::from_value::<AgentTaskManifest>(value).is_err());
    }
}
