//! Shared publication request validation. All entry points, including a
//! future headless CLI, must validate the exact same reviewed file selection.
use std::collections::HashSet;

use infimount_core::CoreError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_AGENT_TASK_PUBLISH_FILES: usize = 100;
pub const MAX_AGENT_TASK_DESTINATION_PATH_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewedAgentTaskOutput {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskPublishConflictPolicy {
    Fail,
    Rename,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskPublicationRequest {
    pub workspace_id: String,
    pub task_id: String,
    pub outputs: Vec<ReviewedAgentTaskOutput>,
    pub destination_storage_id: String,
    #[serde(default)]
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
}

/// Rejects empty selections, malformed paths, non-canonical digests and
/// portable case collisions. Sorting is deterministic for preview fingerprints.
/// Authorization, staleness and write-back decisions happen separately.
pub fn validate_publication_request(
    request: &AgentTaskPublicationRequest,
) -> Result<Vec<ReviewedAgentTaskOutput>, CoreError> {
    Uuid::parse_str(&request.workspace_id).map_err(|_| {
        CoreError::Config("Agent Task publication requires a valid workspace id".into())
    })?;
    Uuid::parse_str(&request.task_id)
        .map_err(|_| CoreError::Config("Agent Task publication requires a valid task id".into()))?;
    if request.destination_storage_id.trim().is_empty() {
        return Err(CoreError::Config(
            "Agent Task publication destination storage is required".into(),
        ));
    }
    if request.outputs.is_empty() || request.outputs.len() > MAX_AGENT_TASK_PUBLISH_FILES {
        return Err(CoreError::Config(format!(
            "Agent Task publication must select between 1 and {MAX_AGENT_TASK_PUBLISH_FILES} outputs"
        )));
    }
    normalize_destination_dir(&request.destination_dir)?;

    let mut outputs = request.outputs.clone();
    outputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));
    let mut seen = HashSet::with_capacity(outputs.len());
    let mut seen_portable = HashSet::with_capacity(outputs.len());
    for output in &outputs {
        validate_output_task_path(&output.task_path)?;
        validate_sha256(&output.sha256)?;
        if !seen.insert(output.task_path.clone())
            || !seen_portable.insert(output.task_path.to_lowercase())
        {
            return Err(CoreError::Config(
                "Agent Task publication contains duplicate or case-colliding output paths".into(),
            ));
        }
    }
    Ok(outputs)
}

pub fn normalize_destination_dir(path: &str) -> Result<String, CoreError> {
    let normalized = path.trim().trim_matches('/');
    if normalized.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES || normalized.contains('\\') {
        return Err(CoreError::Config(
            "publication destination must be a bounded forward-slash path".into(),
        ));
    }
    if normalized.chars().any(|ch| ch == '\0' || ch.is_control())
        || (!normalized.is_empty()
            && normalized
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == ".."))
    {
        return Err(CoreError::Config(
            "publication destination contains an invalid path segment".into(),
        ));
    }
    Ok(normalized.to_string())
}

pub fn validate_output_task_path(path: &str) -> Result<(), CoreError> {
    if path.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES
        || !path.starts_with("outputs/")
        || path.len() <= "outputs/".len()
        || path.contains('\\')
        || path.chars().any(|ch| ch == '\0' || ch.is_control())
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(CoreError::Config(
            "Agent Task publication path must be a safe child of outputs/".into(),
        ));
    }
    Ok(())
}

pub fn validate_sha256(value: &str) -> Result<(), CoreError> {
    if value.len() != 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, 'a'..='f'))
    {
        return Err(CoreError::Config(
            "Agent Task publication requires lowercase SHA-256 fingerprints".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AgentTaskPublicationRequest {
        AgentTaskPublicationRequest {
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            outputs: vec![
                ReviewedAgentTaskOutput {
                    task_path: "outputs/z.txt".into(),
                    byte_size: 7,
                    sha256: "a".repeat(64),
                },
                ReviewedAgentTaskOutput {
                    task_path: "outputs/a.md".into(),
                    byte_size: 9,
                    sha256: "b".repeat(64),
                },
            ],
            destination_storage_id: "destination-id".into(),
            destination_dir: "reports/".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Fail,
        }
    }

    #[test]
    fn deterministic_reviewed_order_and_original_request_preserved() {
        let req = request();
        let outputs = validate_publication_request(&req).unwrap();
        assert_eq!(outputs[0].task_path, "outputs/a.md");
        assert_eq!(outputs[1].task_path, "outputs/z.txt");
        assert_eq!(req.outputs[0].task_path, "outputs/z.txt");
    }

    #[test]
    fn rejects_unselected_or_duplicate_portable_paths() {
        let mut req = request();
        req.outputs.clear();
        assert!(validate_publication_request(&req).is_err());
        req.outputs = request().outputs;
        req.outputs[1].task_path = "outputs/Z.txt".into();
        assert!(validate_publication_request(&req).is_err());
    }

    #[test]
    fn rejects_traversal_and_noncanonical_hash() {
        let mut req = request();
        req.outputs[0].task_path = "outputs/../secret".into();
        assert!(validate_publication_request(&req).is_err());
        req.outputs[0].task_path = "outputs/ok.txt".into();
        req.outputs[0].sha256 = "A".repeat(64);
        assert!(validate_publication_request(&req).is_err());
        assert!(validate_output_task_path("inputs/file").is_err());
        assert!(normalize_destination_dir("reports\\secret").is_err());
        assert!(normalize_destination_dir("reports/../private").is_err());
    }

    #[test]
    fn request_json_is_wire_compatible_and_rejects_new_authority_fields() {
        let req = request();
        let encoded = serde_json::to_value(&req).unwrap();
        assert_eq!(encoded["workspaceId"], req.workspace_id);
        assert_eq!(encoded["destinationDir"], "reports/");
        assert_eq!(encoded["conflictPolicy"], "fail");
        assert_eq!(encoded["outputs"][0]["taskPath"], "outputs/z.txt");
        let decoded: AgentTaskPublicationRequest = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.outputs, req.outputs);

        let mut unsafe_request = encoded;
        unsafe_request["approvePublication"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentTaskPublicationRequest>(unsafe_request).is_err());
    }

    #[test]
    fn path_boundaries_match_legacy_desktop_contract() {
        assert_eq!(
            normalize_destination_dir("/exports/reports/").unwrap(),
            "exports/reports"
        );
        assert!(normalize_destination_dir("reports//inner").is_err());
        assert!(normalize_destination_dir("..").is_err());
        assert!(validate_output_task_path("outputs/report.md").is_ok());
        assert!(validate_output_task_path("outputs/").is_err());
        assert!(validate_output_task_path("outputs\\report.md").is_err());
    }
}
