//! Synthetic, no-provider round-trip for the v0.8.2 C2 hosted path.
//! No credentials, HTTP, storage mounts, or headless publication.
use infimount_control::openai_hosted_artifacts::{
    validate_downloaded_hosted_artifacts, DownloadedHostedArtifact, HostedArtifactInventory,
};
use infimount_control::openai_hosted_events::{
    HostedSessionEventGate, HostedSessionObservedOutcome,
};
use infimount_control::openai_hosted_items::HostedSavedItemAudit;
use infimount_control::openai_hosted_request::{
    build_openai_hosted_offline_request, ReviewedSandboxInputBytes,
};
use infimount_control::sandbox_reviewed_ingest::ingest_reviewed_sandbox_outputs_into_memory;
use infimount_core::agent_tasks::{
    AgentTaskBrief, AgentTaskInput, AgentTaskManifest, AGENT_TASK_SCHEMA_VERSION,
};
use opendal::Operator;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
const TURN_ID: &str = "turn_synthetic";
const SESSION_ID: &str = "sess_synthetic";
const ENVIRONMENT_ID: &str = "env_synthetic";

fn manifest() -> AgentTaskManifest {
    AgentTaskManifest {
        schema_version: AGENT_TASK_SCHEMA_VERSION,
        task_id: TASK_ID.into(),
        title: "PRIVATE original task title".into(),
        created_at: "2026-11-08T12:00:00Z".into(),
        workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
        task_root: format!("tasks/{TASK_ID}"),
        outputs_directory: "outputs".into(),
        inputs: vec![AgentTaskInput {
            task_path: "inputs/sample.csv".into(),
            byte_size: 5,
            sha256: format!("{:x}", Sha256::digest(b"alpha")),
        }],
    }
}

fn reviewed_brief() -> AgentTaskBrief {
    AgentTaskBrief {
        title: "Synthetic calculation".into(),
        objective: "Calculate the approved sample and write the two outputs.".into(),
        requested_outputs: vec!["outputs/summary.json".into(), "outputs/audit.txt".into()],
    }
}

fn item(id: &str, kind: &str, exit: Option<i64>) -> Value {
    match exit {
        Some(code) => json!({
            "id": id, "turn_id": TURN_ID, "type": kind,
            "status": "completed", "exit_code": code
        }),
        None => json!({
            "id": id, "turn_id": TURN_ID, "type": kind,
            "status": "completed", "role": "assistant", "content": []
        }),
    }
}

fn artifact(id: &str, path: &str, byte_size: usize) -> Value {
    json!({
        "id": id, "object": "agent.session.artifact", "turn_id": TURN_ID,
        "session_id": SESSION_ID, "environment_id": ENVIRONMENT_ID,
        "path": path, "size_bytes": byte_size, "created_at": 1780000000
    })
}

fn artifact_page(data: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "object": "list", "has_more": false,
        "first_id": data.first().and_then(|item| item["id"].as_str()),
        "last_id": data.last().and_then(|item| item["id"].as_str()),
        "data": data
    }))
    .unwrap()
}

fn completed_root() -> HostedSessionObservedOutcome {
    let mut events = HostedSessionEventGate::new();
    events
        .observe_json(br#"{"type":"agent.session.created"}"#)
        .unwrap();
    events
        .observe_json(br#"{"type":"agent.session.idle"}"#)
        .unwrap();
    events
        .observe_json(br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":null}}"#)
        .unwrap();
    events.finish_stream()
}

fn audited_root(outcome: HostedSessionObservedOutcome) {
    let mut saved = HostedSavedItemAudit::new(TURN_ID).unwrap();
    let data = vec![
        item("item_input", "message", None),
        item("item_script", "command_execution", Some(0)),
        item("item_answer", "message", None),
    ];
    let page = serde_json::to_vec(&json!({
        "object": "list", "data": data,
        "first_id": "item_input", "last_id": "item_answer", "has_more": false
    }))
    .unwrap();
    saved.observe_page(&page, None).unwrap();
    let summary = saved.finish(outcome).unwrap();
    assert_eq!(summary.items, 3);
    assert_eq!(summary.command_executions, 1);
}

fn memory() -> Operator {
    Operator::new(opendal::services::Memory::default()).unwrap()
}

#[tokio::test]
async fn reviewed_request_root_items_artifacts_hashes_and_create_only_import() {
    let manifest = manifest();
    let brief = reviewed_brief();
    let provided = [ReviewedSandboxInputBytes {
        task_path: "inputs/sample.csv",
        bytes: b"alpha",
    }];
    let request =
        build_openai_hosted_offline_request(&manifest, &brief, &provided, "gpt-6-astra").unwrap();
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["environment"]["network"]["access"], "disabled");
    let input = wire["input"].as_str().unwrap();
    assert!(input.contains("/workspace/outputs/summary.json"));
    assert!(input.contains("/workspace/outputs/audit.txt"));
    assert!(!serde_json::to_string(&wire)
        .unwrap()
        .contains("PRIVATE original task"));

    let outcome = completed_root();
    assert_eq!(
        outcome,
        HostedSessionObservedOutcome::RootTurnCompletedUnverified
    );
    audited_root(outcome);

    let report = br#"{"total": 5}"#;
    let audit = b"pass";
    let mut inventory =
        HostedArtifactInventory::new(&manifest, &brief, SESSION_ID, TURN_ID, ENVIRONMENT_ID)
            .unwrap();
    let page = artifact_page(vec![
        artifact(
            "art_summary",
            "/workspace/outputs/summary.json",
            report.len(),
        ),
        artifact("art_audit", "/workspace/outputs/audit.txt", audit.len()),
    ]);
    inventory.observe_page(&page, None).unwrap();
    let plan = inventory.finish(outcome).unwrap();
    let downloaded = [
        DownloadedHostedArtifact {
            artifact_id: "art_audit",
            bytes: audit,
        },
        DownloadedHostedArtifact {
            artifact_id: "art_summary",
            bytes: report,
        },
    ];
    let candidate = validate_downloaded_hosted_artifacts(&plan, &downloaded).unwrap();

    let staging = memory();
    let original_workspace = memory();
    staging
        .write("outputs/summary.json", report.to_vec())
        .await
        .unwrap();
    staging
        .write("outputs/audit.txt", audit.to_vec())
        .await
        .unwrap();
    let receipt = ingest_reviewed_sandbox_outputs_into_memory(
        &manifest,
        &brief,
        &candidate,
        &staging,
        &original_workspace,
    )
    .await
    .unwrap();
    assert_eq!(receipt.file_count, 2);
    assert_eq!(receipt.total_bytes as usize, report.len() + audit.len());
    assert_eq!(
        original_workspace
            .read(&format!("tasks/{TASK_ID}/outputs/audit.txt"))
            .await
            .unwrap()
            .to_vec(),
        audit
    );
    assert!(ingest_reviewed_sandbox_outputs_into_memory(
        &manifest,
        &brief,
        &candidate,
        &staging,
        &original_workspace,
    )
    .await
    .is_err());
}

#[test]
fn a_completed_turn_with_failed_tool_never_passes_saved_item_gate() {
    let outcome = completed_root();
    let mut saved = HostedSavedItemAudit::new(TURN_ID).unwrap();
    let page = serde_json::to_vec(&json!({
        "object": "list", "has_more": false,
        "first_id": "item_fail", "last_id": "item_fail",
        "data": [item("item_fail", "command_execution", Some(1))]
    }))
    .unwrap();
    assert!(saved.observe_page(&page, None).is_err());
    assert!(saved.finish(outcome).is_err());
}

#[test]
fn the_export_builder_does_not_accept_modified_source_bytes() {
    let provided = [ReviewedSandboxInputBytes {
        task_path: "inputs/sample.csv",
        bytes: b"omega",
    }];
    assert!(build_openai_hosted_offline_request(
        &manifest(),
        &reviewed_brief(),
        &provided,
        "gpt-6-astra"
    )
    .is_err());
}
