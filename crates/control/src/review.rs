//! Shared, read-only Agent Task output review JSON contracts.
//!
//! A review response is evidence for a human decision. It does not include an
//! approval, publish token, destination or implicit authority to write.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskOutputReviewRequest {
    pub workspace_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputReviewFile {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub preview: Option<String>,
    pub preview_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputReviewOutput {
    pub workspace_id: String,
    pub workspace_name: String,
    pub task_id: String,
    pub task_root: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<AgentTaskOutputReviewFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_request_wire_fields_reject_unexpected_authorization() {
        let payload = serde_json::json!({
            "workspaceId": "workspace",
            "taskId": "task"
        });
        let request: AgentTaskOutputReviewRequest =
            serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(request.workspace_id, "workspace");
        assert_eq!(request.task_id, "task");

        let mut unsafe_payload = payload;
        unsafe_payload["approvePublication"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentTaskOutputReviewRequest>(unsafe_payload).is_err());
    }

    #[test]
    fn review_response_json_matches_legacy_desktop_fields_without_publication_state() {
        let response = AgentTaskOutputReviewOutput {
            workspace_id: "workspace".into(),
            workspace_name: "Research".into(),
            task_id: "task".into(),
            task_root: "tasks/task".into(),
            file_count: 2,
            total_bytes: 7,
            files: vec![
                AgentTaskOutputReviewFile {
                    task_path: "outputs/report.md".into(),
                    byte_size: 4,
                    sha256: "a".repeat(64),
                    preview: Some("text".into()),
                    preview_unavailable_reason: None,
                },
                AgentTaskOutputReviewFile {
                    task_path: "outputs/binary.bin".into(),
                    byte_size: 3,
                    sha256: "b".repeat(64),
                    preview: None,
                    preview_unavailable_reason: Some("binary".into()),
                },
            ],
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["workspaceId"], "workspace");
        assert_eq!(json["workspaceName"], "Research");
        assert_eq!(json["taskId"], "task");
        assert_eq!(json["taskRoot"], "tasks/task");
        assert_eq!(json["fileCount"], 2);
        assert_eq!(json["totalBytes"], 7);
        assert_eq!(json["files"][0]["taskPath"], "outputs/report.md");
        assert_eq!(json["files"][0]["byteSize"], 4);
        assert_eq!(json["files"][0]["preview"], "text");
        assert!(json["files"][0]["previewUnavailableReason"].is_null());
        assert_eq!(json["files"][1]["previewUnavailableReason"], "binary");
        assert!(json.get("approved").is_none());
        assert!(json.get("destinationStorageId").is_none());
        assert!(json.get("previewToken").is_none());
    }
}
