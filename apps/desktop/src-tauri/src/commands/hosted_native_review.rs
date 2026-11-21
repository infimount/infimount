//! Linux native scope dialog. This is PRESENTATION ONLY, not an execution
//! consent issuer, an auth factor, an IPC endpoint or permission to spend.
//!
//! The trusted desktop caller must hold lifecycle/configuration/workspace
//! mutation locks, rehash actual prepared bytes and independently bind any
//! approval to a fresh one-use native consent capability. A webview boolean
//! or this dialog's response alone must never authorize upload or execution.
use std::cell::RefCell;

use gtk::prelude::*;
use infimount_control::sandbox_bundle::SandboxTaskBundle;
use infimount_control::sandbox_local_export::VerifiedHostedInputScope;
use infimount_core::CoreError;
use tauri::Manager;

const MAX_DIALOG_CHARS: usize = 4200;
const MAX_PILOT_FILES: usize = 4;
const MAX_PILOT_OUTPUTS: usize = 4;
const MAX_INSTRUCTION_CHARS: usize = 1600;

fn reject() -> CoreError {
    CoreError::Config("ERR_HOSTED_NATIVE_REVIEW_UNAVAILABLE".into())
}

fn plain_reviewed_text(text: &str, limit: usize) -> bool {
    !text.is_empty()
        && text.len() <= limit
        && text
            .bytes()
            .all(|c| c == b'\n' || c == b'\t' || (0x20..=0x7e).contains(&c))
}

fn indent_lines(text: &str) -> String {
    text.lines()
        .map(|line| format!("  | {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Complete, independently reviewed scope only. In the first synthetic pilot,
/// refuse a larger scope that would not fit an honest native modal display.
pub(crate) fn reviewed_native_message(
    bundle: &SandboxTaskBundle,
    verified: &VerifiedHostedInputScope,
) -> Result<String, CoreError> {
    if bundle.inputs.len() > MAX_PILOT_FILES
        || bundle.requested_outputs.is_empty()
        || bundle.requested_outputs.len() > MAX_PILOT_OUTPUTS
        || bundle.inputs.len() != verified.input_file_count
        || bundle.input_total_bytes != verified.input_total_bytes
        || bundle.bundle_metadata_sha256 != verified.bundle_metadata_sha256
        || !verified.verified_bytes
        || verified.export_authorized
        || !plain_reviewed_text(&bundle.title, 160)
        || !plain_reviewed_text(&bundle.instructions, MAX_INSTRUCTION_CHARS)
        || verified.bundle_metadata_sha256.len() != 64
        || !verified
            .bundle_metadata_sha256
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(reject());
    }
    let mut message = format!(
        "UPLOAD TO EXTERNAL PROVIDER, ONE RUN ONLY\n\n\
         Reviewed task: {}\n\n\
         Reviewed/redacted instructions:\n{}\n\n\
         Files that will leave this device:\n",
        bundle.title,
        indent_lines(&bundle.instructions),
    );
    for file in &bundle.inputs {
        if !plain_reviewed_text(&file.task_path, 256)
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(reject());
        }
        message.push_str(&format!(
            "  {}  ({} bytes, SHA-256 {})\n",
            file.task_path, file.byte_size, file.sha256,
        ));
    }
    message.push_str("\nRequested output paths:\n");
    for output in &bundle.requested_outputs {
        if !plain_reviewed_text(output, 256) {
            return Err(reject());
        }
        message.push_str(&format!("  {output}\n"));
    }
    message.push_str(&format!(
        "\nVerified review digest:\n{}\n\n\
         Provider: OpenAI Agents API, U.S. only, non-ZDR.\n\
         Contents and instructions may be retained by the provider.\n\
         External network in hosted compute: DISABLED.\n\
         Remote charges: US$1 best-effort limit, NOT a guaranteed cap.\n\
         Credential configured is NOT authorization to spend.\n\n\
         Approve this exact export and one hosted execution?",
        verified.bundle_metadata_sha256,
    ));
    if message.len() > MAX_DIALOG_CHARS {
        return Err(reject());
    }
    Ok(message)
}

/// Explicit native OS dialog, parented to the main window. Never call this
/// from headless/MCP contexts. On window close/error/missing main window the
/// answer is NO. The caller must compare the rehashed snapshot again after
/// the dialog, before issuing a one-use execution capability.
#[allow(dead_code)]
pub(crate) async fn ask_native_hosted_scope_confirmation(
    app: &tauri::AppHandle,
    bundle: &SandboxTaskBundle,
    verified: &VerifiedHostedInputScope,
) -> Result<bool, CoreError> {
    let message = reviewed_native_message(bundle, verified)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let parent = handle
            .get_webview_window("main")
            .and_then(|window| window.gtk_window().ok());
        let Some(parent) = parent else {
            let _ = sender.send(false);
            return;
        };
        let dialog = gtk::MessageDialog::new(
            Some(&parent),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Warning,
            gtk::ButtonsType::None,
            &message,
        );
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Approve one hosted run", gtk::ResponseType::Ok);
        dialog.set_default_response(gtk::ResponseType::Cancel);
        let once = RefCell::new(Some(sender));
        dialog.connect_response(move |dialog, response| {
            if let Some(sender) = once.borrow_mut().take() {
                let _ = sender.send(response == gtk::ResponseType::Ok);
            }
            // SAFETY: this is the final use of the live GTK dialog in the
            // response callback. The dialog is not accessed after destruction.
            unsafe { dialog.destroy() };
        });
        dialog.show_all();
    })
    .map_err(|_| reject())?;
    receiver.await.map_err(|_| reject())
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_tasks::AgentTaskInput;

    fn fixture() -> (SandboxTaskBundle, VerifiedHostedInputScope) {
        let bundle = SandboxTaskBundle {
            schema_version: 1,
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            title: "Summarize public synthetic fixture".into(),
            instructions: "Read synthetic data.\nWrite a short summary.".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/public-sample.txt".into(),
                byte_size: 12,
                sha256: "a".repeat(64),
            }],
            outputs_directory: "outputs".into(),
            requested_outputs: vec!["outputs/summary.md".into()],
            input_total_bytes: 12,
            bundle_metadata_sha256: "b".repeat(64),
        };
        let verified = VerifiedHostedInputScope {
            input_file_count: 1,
            input_total_bytes: 12,
            bundle_metadata_sha256: "b".repeat(64),
            verified_bytes: true,
            export_authorized: false,
        };
        (bundle, verified)
    }

    #[test]
    fn native_message_discloses_exact_files_paths_instructions_and_provider_terms() {
        let (bundle, verified) = fixture();
        let msg = reviewed_native_message(&bundle, &verified).unwrap();
        for expected in [
            "public-sample.txt",
            "12 bytes",
            "Read synthetic data.",
            "outputs/summary.md",
            "non-ZDR",
            "U.S. only",
            "best-effort limit",
            "not a guaranteed cap", // case check below
        ] {
            assert!(
                msg.to_lowercase().contains(&expected.to_lowercase()),
                "{expected}"
            );
        }
        assert!(!msg.contains("/home/"));
        assert!(!msg.contains("sk-proj"));
        assert!(!msg.contains(&bundle.task_id));
    }

    #[test]
    fn native_message_rejects_changed_scope_unauthorized_or_huge_display() {
        let (mut bundle, mut verified) = fixture();
        verified.bundle_metadata_sha256 = "c".repeat(64);
        assert!(reviewed_native_message(&bundle, &verified).is_err());
        verified.bundle_metadata_sha256 = "b".repeat(64);
        verified.export_authorized = true;
        assert!(reviewed_native_message(&bundle, &verified).is_err());
        verified.export_authorized = false;
        bundle.instructions = "x".repeat(MAX_INSTRUCTION_CHARS + 1);
        assert!(reviewed_native_message(&bundle, &verified).is_err());
        bundle.instructions = "ok".into();
        bundle.inputs[0].task_path = "inputs/a\nAPPROVE WITHOUT READING".into();
        assert!(reviewed_native_message(&bundle, &verified).is_err());
    }
}
