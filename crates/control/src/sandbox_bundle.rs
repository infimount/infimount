//! Provider-neutral, metadata-only Agent Task export contract for remote sandboxes.
//!
//! This module DOES NOT upload files, load source storages, authenticate users,
//! or grant publication permissions. The caller must show the export contents
//! to the user and explicitly obtain a reviewed, redacted task brief. Original
//! workspace/storage metadata and the unreviewed TASK.md are never copied here.
//!
//! The digest commits to the selected metadata and DECLARED file hashes. Before
//! uploading a bundle, a later staging service must rehash actual prepared bytes
//! under the task's trusted boundary and compare those hashes again.
use infimount_core::agent_tasks::{
    validate_agent_task_brief, validate_agent_task_manifest, AgentTaskBrief, AgentTaskInput,
    AgentTaskManifest, AGENT_TASK_OUTPUTS_DIR, MAX_AGENT_TASK_PREPARED_BYTES,
};
use infimount_core::CoreError;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const SANDBOX_TASK_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const SANDBOX_TASK_BUNDLE_DOMAIN: &str = "infimount:v0.8.2:sandbox-task-bundle:v1";
pub const MAX_SANDBOX_BUNDLE_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// Only this explicitly enumerated subset may cross the provider boundary.
/// No workspace identifier, source storage, local host path, credentials,
/// MCP policy, destination publication target, or execution state is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxTaskBundle {
    pub schema_version: u32,
    pub task_id: String,
    /// Reviewed export title, not automatically copied from the local task.
    pub title: String,
    /// Reviewed/redacted export instructions, not automatically TASK.md.
    pub instructions: String,
    pub inputs: Vec<AgentTaskInput>,
    pub outputs_directory: String,
    pub requested_outputs: Vec<String>,
    pub input_total_bytes: u64,
    /// SHA-256 over the versioned, domain-separated metadata contract.
    /// This is NOT proof that the source file bytes were rehashed for upload.
    pub bundle_aggregate_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedSandboxBundle<'a> {
    domain: &'static str,
    schema_version: u32,
    task_id: &'a str,
    title: &'a str,
    instructions: &'a str,
    inputs: &'a [AgentTaskInput],
    outputs_directory: &'static str,
    requested_outputs: &'a [String],
    input_total_bytes: u64,
}

/// Build deterministic export metadata from a valid prepared task and a
/// SEPARATELY reviewed, redacted brief for an external sandbox.
///
/// This function intentionally never reads or transfers any file data. It
/// cannot detect arbitrary secrets in free-form instructions. Callers must
/// present and approve those bytes before uploading to a remote provider.
pub fn build_sandbox_task_bundle(
    manifest: &AgentTaskManifest,
    reviewed_export_brief: &AgentTaskBrief,
) -> Result<SandboxTaskBundle, CoreError> {
    validate_agent_task_manifest(manifest)?;
    validate_agent_task_brief(reviewed_export_brief)?;

    if reviewed_export_brief.title.trim() != reviewed_export_brief.title
        || reviewed_export_brief.objective.trim() != reviewed_export_brief.objective
        || reviewed_export_brief.objective.chars().any(|ch| {
            ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')
        })
    {
        return Err(CoreError::Config(
            "sandbox export requires canonical, reviewed title and instructions".into(),
        ));
    }

    let input_total_bytes = manifest.inputs.iter().try_fold(0_u64, |sum, item| {
        sum.checked_add(item.byte_size)
            .ok_or_else(|| CoreError::Config("sandbox input byte count overflowed".into()))
    })?;
    if input_total_bytes > MAX_AGENT_TASK_PREPARED_BYTES {
        return Err(CoreError::Config(
            "sandbox inputs exceed the prepared task byte limit".into(),
        ));
    }

    let mut inputs = manifest.inputs.clone();
    inputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));
    let mut requested_outputs = reviewed_export_brief.requested_outputs.clone();
    requested_outputs.sort();

    let unsigned = UnsignedSandboxBundle {
        domain: SANDBOX_TASK_BUNDLE_DOMAIN,
        schema_version: SANDBOX_TASK_BUNDLE_SCHEMA_VERSION,
        task_id: &manifest.task_id,
        title: &reviewed_export_brief.title,
        instructions: &reviewed_export_brief.objective,
        inputs: &inputs,
        outputs_directory: AGENT_TASK_OUTPUTS_DIR,
        requested_outputs: &requested_outputs,
        input_total_bytes,
    };
    let bytes = serde_json::to_vec(&unsigned)?;
    if bytes.len() > MAX_SANDBOX_BUNDLE_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "sandbox metadata manifest exceeds the 4 MiB limit".into(),
        ));
    }
    Ok(SandboxTaskBundle {
        schema_version: SANDBOX_TASK_BUNDLE_SCHEMA_VERSION,
        task_id: manifest.task_id.clone(),
        title: reviewed_export_brief.title.clone(),
        instructions: reviewed_export_brief.objective.clone(),
        inputs,
        outputs_directory: AGENT_TASK_OUTPUTS_DIR.to_owned(),
        requested_outputs,
        input_total_bytes,
        bundle_aggregate_sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: 1,
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            title: "PRIVATE: internal storage migration audit".into(),
            created_at: "2026-10-29T12:00:00Z".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_root: "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            outputs_directory: "outputs".into(),
            inputs: vec![
                AgentTaskInput {
                    task_path: "inputs/z.txt".into(),
                    byte_size: 7,
                    sha256: "b".repeat(64),
                },
                AgentTaskInput {
                    task_path: "inputs/a.md".into(),
                    byte_size: 5,
                    sha256: "a".repeat(64),
                },
            ],
        }
    }

    fn safe_review() -> AgentTaskBrief {
        AgentTaskBrief {
            title: "Summarize sample data".into(),
            objective: "Analyze the two approved inputs and write an overview.".into(),
            requested_outputs: vec!["outputs/summary.md".into(), "outputs/chart.csv".into()],
        }
    }

    #[test]
    fn export_contains_only_explicit_reviewed_metadata_and_no_host_identity() {
        let bundle = build_sandbox_task_bundle(&task(), &safe_review()).unwrap();
        assert_eq!(bundle.input_total_bytes, 12);
        assert_eq!(bundle.inputs[0].task_path, "inputs/a.md");
        assert_eq!(bundle.requested_outputs[0], "outputs/chart.csv");
        assert_eq!(bundle.bundle_aggregate_sha256.len(), 64);
        // Cross-runtime canonical v1 wire vector, independently SHA-256 hashed.
        assert_eq!(
            bundle.bundle_aggregate_sha256,
            "308bdae03cc5e13da6a10dcb5d755d1bb83e8ae614059a92a16949e184a75971"
        );

        let json = serde_json::to_value(&bundle).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["outputsDirectory"], "outputs");
        assert_eq!(json["title"], "Summarize sample data");
        for field in [
            "workspaceId",
            "taskRoot",
            "sourceStorageId",
            "storageName",
            "destinationStorageId",
            "mcpPolicy",
            "secretRef",
            "hostPath",
            "createdAt",
        ] {
            assert!(json.get(field).is_none(), "{field} must never be exported");
        }
        let encoded = serde_json::to_string(&bundle).unwrap();
        assert!(!encoded.contains("PRIVATE:"));
        assert!(!encoded.contains("f8f47aa7-702d-4fd6-8815-84cbdf3b3127"));
    }

    #[test]
    fn bundle_digest_is_deterministic_and_binds_reviewed_bytes() {
        let initial = build_sandbox_task_bundle(&task(), &safe_review()).unwrap();
        let mut reordered = task();
        reordered.inputs.reverse();
        let mut reviewed = safe_review();
        reviewed.requested_outputs.reverse();
        assert_eq!(
            initial,
            build_sandbox_task_bundle(&reordered, &reviewed).unwrap()
        );

        reordered.inputs[0].sha256 = "c".repeat(64);
        let changed_source = build_sandbox_task_bundle(&reordered, &reviewed).unwrap();
        assert_ne!(
            initial.bundle_aggregate_sha256,
            changed_source.bundle_aggregate_sha256
        );

        reviewed.objective.push_str(" Add a short risks section.");
        let changed_review = build_sandbox_task_bundle(&task(), &reviewed).unwrap();
        assert_ne!(
            initial.bundle_aggregate_sha256,
            changed_review.bundle_aggregate_sha256
        );
    }

    #[test]
    fn rejects_invalid_prepared_scope_unreviewed_controls_and_excessive_bytes() {
        let mut bad = task();
        bad.inputs[0].task_path = "inputs/../secrets".into();
        assert!(build_sandbox_task_bundle(&bad, &safe_review()).is_err());

        let mut bad = task();
        bad.inputs[0].task_path = "inputs/A.md".into();
        assert!(build_sandbox_task_bundle(&bad, &safe_review()).is_err());

        let mut huge = task();
        huge.inputs[0].byte_size = MAX_AGENT_TASK_PREPARED_BYTES;
        assert!(build_sandbox_task_bundle(&huge, &safe_review()).is_err());

        let mut unsafe_brief = safe_review();
        unsafe_brief.objective.push('\u{001b}');
        assert!(build_sandbox_task_bundle(&task(), &unsafe_brief).is_err());

        unsafe_brief = safe_review();
        unsafe_brief.objective.insert(0, ' ');
        assert!(build_sandbox_task_bundle(&task(), &unsafe_brief).is_err());
    }
}
