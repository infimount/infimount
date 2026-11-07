//! Offline, allowlisted OpenAI-managed Agents API sandbox request builder.
//!
//! This module performs NO HTTP call, does not retrieve credentials, does not
//! create a session, and grants no storage or publication permission.
//! The caller must independently obtain explicit user consent before sending
//! any request or paying a provider. Agent prompt text is NOT a security policy.
//!
//! OpenAI-hosted sandboxes default to outbound networking when unspecified.
//! This contract always emits environment.network.access = "disabled", with
//! no mounts, provider env/secrets, setup commands, external MCP or source IDs.
use base64::Engine as _;
use infimount_core::agent_tasks::{AgentTaskBrief, AgentTaskManifest, MAX_AGENT_TASK_PATH_LEN};
use infimount_core::CoreError;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::sandbox_bundle::build_sandbox_task_bundle;

pub const OPENAI_HOSTED_INLINE_MAX_FILES: usize = 32;
pub const OPENAI_HOSTED_INLINE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const OPENAI_HOSTED_REQUEST_MAX_JSON_BYTES: usize = 12 * 1024 * 1024;

const FIXED_AGENT_INSTRUCTIONS: &str =
    "Use only the provided workspace inputs. Write deliverables under /workspace/outputs/. Do not seek source storage credentials or change the destination.";

/// Tell the hosted agent the exact separately reviewed deliverables.
/// These are task instructions, not filesystem or publication permissions.
fn render_hosted_input(instructions: &str, requested_outputs: &[String]) -> String {
    let mut input = instructions.to_owned();
    if !requested_outputs.is_empty() {
        input.push_str("\n\nApproved deliverables (paths inside /workspace):\n");
        for output in requested_outputs {
            input.push_str("- /workspace/");
            input.push_str(output);
            input.push('\n');
        }
    }
    input
}

pub struct ReviewedSandboxInputBytes<'a> {
    pub task_path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Serialize)]
struct HostedAgent {
    model: String,
    instructions: &'static str,
}

#[derive(Serialize)]
struct HostedNetwork {
    access: &'static str,
}

#[derive(Serialize)]
struct HostedInlineFile {
    #[serde(rename = "type")]
    file_type: &'static str,
    path: String,
    data: String,
}

#[derive(Serialize)]
struct HostedEnvironment {
    #[serde(rename = "type")]
    environment_type: &'static str,
    network: HostedNetwork,
    files: Vec<HostedInlineFile>,
}

/// Only an opaque, serializable request. It contains reviewed bytes, so do
/// not derive Debug and never log its JSON or hand it to a model tool.
#[derive(Serialize)]
pub struct OpenAiHostedOfflineSessionRequest {
    agent: HostedAgent,
    environment: HostedEnvironment,
    input: String,
    stream: bool,
}

/// Verify bytes against the trusted prepared-task manifest before constructing
/// the OpenAI-hosted session request. No HTTP call or API key is involved.
///
/// This narrowly supports small inline-file pilots. Larger bundles will need
/// an explicit File-ID upload stage and byte-verification evidence.
pub fn build_openai_hosted_offline_request(
    manifest: &AgentTaskManifest,
    reviewed_export_brief: &AgentTaskBrief,
    provided_inputs: &[ReviewedSandboxInputBytes<'_>],
    model_id: &str,
) -> Result<OpenAiHostedOfflineSessionRequest, CoreError> {
    if model_id.is_empty()
        || model_id.len() > 100
        || !model_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CoreError::Config(
            "sandbox model identifier is not canonical".into(),
        ));
    }

    // This builder owns the construction of the reviewed export projection
    // rather than trusting a mutable serialized bundle from the provider.
    let bundle = build_sandbox_task_bundle(manifest, reviewed_export_brief)?;
    if bundle.inputs.len() != provided_inputs.len()
        || bundle.inputs.len() > OPENAI_HOSTED_INLINE_MAX_FILES
    {
        return Err(CoreError::Config(
            "sandbox inline files differ from reviewed input selection".into(),
        ));
    }

    let mut sorted = provided_inputs.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.task_path.cmp(right.task_path));
    let mut total_bytes = 0_usize;
    for (given, expected) in sorted.iter().zip(&bundle.inputs) {
        if given.task_path != expected.task_path
            || given.task_path.len() > MAX_AGENT_TASK_PATH_LEN
            || given.bytes.len() as u64 != expected.byte_size
            || format!("{:x}", Sha256::digest(given.bytes)) != expected.sha256
        {
            return Err(CoreError::Config(
                "sandbox inline input bytes differ from reviewed prepared file".into(),
            ));
        }
        total_bytes = total_bytes
            .checked_add(given.bytes.len())
            .ok_or_else(|| CoreError::Config("sandbox inline input size overflowed".into()))?;
        if total_bytes > OPENAI_HOSTED_INLINE_MAX_BYTES {
            return Err(CoreError::Config(
                "sandbox inline inputs exceed the 8 MiB pilot limit".into(),
            ));
        }
    }
    if total_bytes as u64 != bundle.input_total_bytes {
        return Err(CoreError::Config(
            "sandbox inline input total differs from reviewed bundle".into(),
        ));
    }

    let files = sorted
        .iter()
        .map(|file| HostedInlineFile {
            file_type: "inline",
            path: format!("/workspace/{}", file.task_path),
            data: base64::engine::general_purpose::STANDARD.encode(file.bytes),
        })
        .collect();

    let request = OpenAiHostedOfflineSessionRequest {
        agent: HostedAgent {
            model: model_id.to_owned(),
            instructions: FIXED_AGENT_INSTRUCTIONS,
        },
        environment: HostedEnvironment {
            environment_type: "openai_hosted",
            network: HostedNetwork { access: "disabled" },
            files,
        },
        input: render_hosted_input(&bundle.instructions, &bundle.requested_outputs),
        stream: true,
    };

    if serde_json::to_vec(&request)?.len() > OPENAI_HOSTED_REQUEST_MAX_JSON_BYTES {
        return Err(CoreError::Config(
            "sandbox inline API request exceeds its bounded wire envelope".into(),
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};

    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
    const WORKSPACE_ID: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";

    fn manifest(contents: &[u8]) -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: TASK_ID.into(),
            title: "PRIVATE host title: internal storage audit".into(),
            created_at: "2026-11-07T12:00:00Z".into(),
            workspace_id: WORKSPACE_ID.into(),
            task_root: format!("tasks/{TASK_ID}"),
            outputs_directory: "outputs".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/sample.csv".into(),
                byte_size: contents.len() as u64,
                sha256: format!("{:x}", Sha256::digest(contents)),
            }],
        }
    }

    fn reviewed() -> AgentTaskBrief {
        AgentTaskBrief {
            title: "Summarize approved sample data".into(),
            objective: "Summarize only sample.csv into outputs/summary.md.".into(),
            requested_outputs: vec!["outputs/summary.md".into()],
        }
    }

    #[test]
    fn offline_request_is_network_denied_and_contains_no_original_storage_identity() {
        let inputs = [ReviewedSandboxInputBytes {
            task_path: "inputs/sample.csv",
            bytes: b"alpha",
        }];
        let request = build_openai_hosted_offline_request(
            &manifest(b"alpha"),
            &reviewed(),
            &inputs,
            "gpt-6-astra",
        )
        .unwrap();
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["agent"]["model"], "gpt-6-astra");
        assert_eq!(json["environment"]["type"], "openai_hosted");
        assert_eq!(json["environment"]["network"]["access"], "disabled");
        assert_eq!(json["environment"]["files"][0]["type"], "inline");
        assert_eq!(
            json["environment"]["files"][0]["path"],
            "/workspace/inputs/sample.csv"
        );
        assert_eq!(json["environment"]["files"][0]["data"], "YWxwaGE=");
        assert_eq!(
            json["input"],
            concat!(
                "Summarize only sample.csv into outputs/summary.md.",
                "\n\nApproved deliverables (paths inside /workspace):\n",
                "- /workspace/outputs/summary.md\n",
            )
        );
        assert_eq!(json["stream"], true);
        for key in [
            "env",
            "setup_commands",
            "packages",
            "mounts",
            "mcp",
            "credentials",
        ] {
            assert!(
                json["environment"].get(key).is_none(),
                "{key} must be absent"
            );
        }
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("PRIVATE host title"));
        assert!(!encoded.contains(WORKSPACE_ID));
        assert!(!encoded.contains("secretRef"));
        assert!(!encoded.contains("api_key"));
    }

    #[test]
    fn same_size_changed_bytes_and_missing_or_extra_files_are_rejected() {
        let task = manifest(b"alpha");
        let brief = reviewed();
        let changed = [ReviewedSandboxInputBytes {
            task_path: "inputs/sample.csv",
            bytes: b"omega",
        }];
        assert!(
            build_openai_hosted_offline_request(&task, &brief, &changed, "gpt-6-astra").is_err()
        );
        assert!(build_openai_hosted_offline_request(&task, &brief, &[], "gpt-6-astra").is_err());
        let extra = [
            ReviewedSandboxInputBytes {
                task_path: "inputs/sample.csv",
                bytes: b"alpha",
            },
            ReviewedSandboxInputBytes {
                task_path: "inputs/extra.csv",
                bytes: b"private",
            },
        ];
        assert!(build_openai_hosted_offline_request(&task, &brief, &extra, "gpt-6-astra").is_err());
    }

    #[test]
    fn reviewed_deliverable_paths_are_sorted_and_included_in_model_task() {
        let prepared = manifest(b"alpha");
        let inputs = [ReviewedSandboxInputBytes {
            task_path: "inputs/sample.csv",
            bytes: b"alpha",
        }];
        let mut brief = reviewed();
        brief.requested_outputs = vec!["outputs/z.md".into(), "outputs/a.md".into()];
        let first =
            build_openai_hosted_offline_request(&prepared, &brief, &inputs, "gpt-6-astra").unwrap();
        brief.requested_outputs.reverse();
        let second =
            build_openai_hosted_offline_request(&prepared, &brief, &inputs, "gpt-6-astra").unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        let serialized = serde_json::to_value(&first).unwrap();
        let input = serialized["input"].as_str().unwrap();
        assert!(input.contains("- /workspace/outputs/a.md\n"));
        assert!(input.contains("- /workspace/outputs/z.md\n"));
        assert!(input.find("outputs/a.md").unwrap() < input.find("outputs/z.md").unwrap());

        brief.requested_outputs = vec!["outputs/../secret".into()];
        assert!(
            build_openai_hosted_offline_request(&prepared, &brief, &inputs, "gpt-6-astra",)
                .is_err()
        );
    }

    #[test]
    fn rejects_invalid_model_unreviewed_instructions_and_excess_inline_bytes() {
        let task = manifest(b"alpha");
        let input = [ReviewedSandboxInputBytes {
            task_path: "inputs/sample.csv",
            bytes: b"alpha",
        }];
        for model in ["", "../unsafe", "model with spaces", "x/y", "x\ncommand"] {
            assert!(
                build_openai_hosted_offline_request(&task, &reviewed(), &input, model).is_err()
            );
        }
        let mut brief = reviewed();
        brief.objective = " unreviewed whitespace ".into();
        assert!(build_openai_hosted_offline_request(&task, &brief, &input, "gpt-6-astra").is_err());

        let oversized = vec![b'x'; OPENAI_HOSTED_INLINE_MAX_BYTES + 1];
        let task = manifest(&oversized);
        let file = [ReviewedSandboxInputBytes {
            task_path: "inputs/sample.csv",
            bytes: &oversized,
        }];
        assert!(
            build_openai_hosted_offline_request(&task, &reviewed(), &file, "gpt-6-astra").is_err()
        );
    }
}
