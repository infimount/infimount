//! Supervised process execution for an eventual Claude Code Agent Task adapter.
//! This module is not exposed through Tauri until real packaged-client
//! qualification proves the CLI's MCP and restriction semantics.
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use infimount_core::CoreError;

use crate::agent_adapters::{executable_digest, verify_executable_identity, ExecutableIdentity};
use crate::client_integrations::run_with_timeout;

const MAX_RUNTIME: Duration = Duration::from_secs(12 * 60);
const STDOUT_LIMIT: usize = 64 * 1024;
const STDERR_LIMIT: usize = 8 * 1024;
const READ_BLOCK: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl ClaudeOutcome {
    pub(crate) fn code(self) -> Option<&'static str> {
        match self {
            Self::Succeeded => None,
            Self::Failed => Some("ERR_CLAUDE_EXECUTION_FAILED"),
            Self::Cancelled => Some("ERR_CLAUDE_CANCELLED"),
            Self::TimedOut => Some("ERR_CLAUDE_TIMED_OUT"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SupervisedOutcome {
    pub(crate) result: ClaudeOutcome,
    /// Indicates a CLI *process* completed and produced JSON with is_error=false.
    /// This is never a claim about the quality of the generated task outputs.
    pub(crate) exit_code: Option<i32>,
}

#[derive(Default)]
struct BoundedCapture {
    content: Vec<u8>,
    exceeded: bool,
}

fn capture<R: Read>(mut stream: R, limit: usize) -> std::io::Result<BoundedCapture> {
    let mut result = BoundedCapture::default();
    let mut block = [0_u8; READ_BLOCK];
    loop {
        let size = stream.read(&mut block)?;
        if size == 0 {
            break;
        }
        let remaining = limit.saturating_sub(result.content.len());
        result
            .content
            .extend_from_slice(&block[..size.min(remaining)]);
        result.exceeded |= size > remaining;
    }
    Ok(result)
}

/// Supply the minimum variables needed for a user's existing Claude login.
/// Do not inherit arbitrary parent environment keys or cloud credentials.
fn scoped_environment(command: &mut Command) {
    command.env_clear();
    for name in ["HOME", "PATH", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("TERM", "dumb");
    command.env("NO_COLOR", "1");
}

/// Read-only client probe. The version must be compatible with the documented
/// fail-closed CLI flags, and the binary digest is revalidated before execution.
pub(crate) fn detect_claude_at(path: &Path) -> Result<ExecutableIdentity, CoreError> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| CoreError::Config("Claude Code executable is unavailable".into()))?;
    if !canonical.is_absolute() {
        return Err(CoreError::Config(
            "Claude Code executable must be absolute".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&canonical)?.permissions().mode() & 0o111 == 0 {
            return Err(CoreError::Config(
                "Claude Code executable is not executable".into(),
            ));
        }
    }
    let sha256 = executable_digest(&canonical)?;
    let args = vec!["--version".to_owned()];
    let (status, output, _, timed_out) =
        run_with_timeout(&canonical, &args, Duration::from_secs(3), 4096);
    if timed_out || !status.is_some_and(|s| s.success()) {
        return Err(CoreError::Config("Claude Code version probe failed".into()));
    }
    let reported_version = output.trim().to_string();
    crate::claude_handoff::require_qualified_flags(&reported_version)?;
    let identity = ExecutableIdentity {
        executable: canonical,
        sha256,
        reported_version,
    };
    verify_executable_identity(&identity)?;
    Ok(identity)
}

fn verified_cwd(path: &Path) -> Result<(), CoreError> {
    if !path.is_absolute() {
        return Err(CoreError::Config(
            "Claude neutral working directory must be absolute".into(),
        ));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CoreError::Config("Claude neutral working directory is missing".into()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::Config(
            "Claude neutral working directory is not an ordinary directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(CoreError::Config(
                "Claude neutral working directory must be private".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_owned_group(child: &mut std::process::Child) {
    let group_id = child.id();
    if group_id <= i32::MAX as u32 {
        // The child was launched in a new process group, not the desktop group.
        unsafe {
            libc::kill(-(group_id as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Executes an exact internally built Claude argument vector (see claude_argv).
/// Does not shell out, expose paths to the model, or retain raw model output.
/// The current implementation only supports Unix process-group isolation.
/// Windows is denied until job-object launch and cancellation are qualified.
pub(crate) fn execute_verified_claude(
    identity: &ExecutableIdentity,
    args: &[String],
    cwd: &Path,
    cancelled: &AtomicBool,
    max_runtime: Duration,
) -> Result<SupervisedOutcome, CoreError> {
    verify_executable_identity(identity)?;
    verified_cwd(cwd)?;
    if max_runtime.is_zero() || max_runtime > MAX_RUNTIME {
        return Err(CoreError::Config(
            "Claude execution runtime limit is invalid".into(),
        ));
    }
    if cancelled.load(Ordering::SeqCst) {
        return Ok(SupervisedOutcome {
            result: ClaudeOutcome::Cancelled,
            exit_code: None,
        });
    }
    #[cfg(not(unix))]
    {
        let _ = (args, identity);
        return Err(CoreError::Config(
            "Claude execution requires qualified process-tree containment on this platform".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new(&identity.executable);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        scoped_environment(&mut command);
        verify_executable_identity(identity)?;
        let mut child = command.spawn().map_err(|_| {
            CoreError::Config("verified Claude executable could not be launched".into())
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CoreError::Config("Claude output pipe is unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CoreError::Config("Claude error pipe is unavailable".into()))?;
        let out_thread = thread::spawn(move || capture(stdout, STDOUT_LIMIT));
        let err_thread = thread::spawn(move || capture(stderr, STDERR_LIMIT));

        let start = Instant::now();
        let (code, reason) = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (status.code(), None),
                Ok(None) if cancelled.load(Ordering::SeqCst) => {
                    break (None, Some(ClaudeOutcome::Cancelled))
                }
                Ok(None) if start.elapsed() >= max_runtime => {
                    break (None, Some(ClaudeOutcome::TimedOut))
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => break (None, Some(ClaudeOutcome::Failed)),
            }
        };
        // A CLI child may have spawned descendants keeping pipes open. Kill
        // the dedicated group before joining capture threads on every exit.
        terminate_owned_group(&mut child);
        let stdout = out_thread
            .join()
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let stderr = err_thread
            .join()
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let _ = stderr; // Diagnostics are never persisted or reflected to the UI.
        let result = if let Some(reason) = reason {
            reason
        } else if code != Some(0) || stdout.exceeded {
            ClaudeOutcome::Failed
        } else {
            let response: serde_json::Value = match serde_json::from_slice(&stdout.content) {
                Ok(response) => response,
                Err(_) => serde_json::Value::Null,
            };
            if response.get("is_error") == Some(&serde_json::Value::Bool(false)) {
                ClaudeOutcome::Succeeded
            } else {
                ClaudeOutcome::Failed
            }
        };
        Ok(SupervisedOutcome {
            result,
            exit_code: code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_adapters::ExecutableIdentity;
    use sha2::{Digest, Sha256};

    fn private_tempdir() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        temp
    }

    #[cfg(unix)]
    fn mock_cli(dir: &Path, body: &str) -> ExecutableIdentity {
        use std::os::unix::fs::PermissionsExt;
        let executable = dir.join("claude-mock");
        fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let canonical = fs::canonicalize(executable).unwrap();
        let sha256 = format!("{:x}", Sha256::digest(fs::read(&canonical).unwrap()));
        ExecutableIdentity {
            executable: canonical,
            sha256,
            reported_version: "2.1.259 (Claude Code)".into(),
        }
    }

    #[test]
    fn rejects_non_private_or_relative_cwd() {
        assert!(verified_cwd(Path::new("relative/path")).is_err());
        let dir = private_tempdir();
        assert!(verified_cwd(dir.path()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn captures_success_without_leaking_output_or_parent_environment() {
        let temp = private_tempdir();
        let identity = mock_cli(
            temp.path(),
            "if [ -n \"$INFIMOUNT_SECRET_MARKER\" ]; then exit 4; fi\nprintf '%s' '{\"is_error\":false,\"result\":\"ok\"}'",
        );
        let outcome = execute_verified_claude(
            &identity,
            &[],
            temp.path(),
            &AtomicBool::new(false),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(outcome.result, ClaudeOutcome::Succeeded);
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[test]
    fn inherited_agent_secrets_are_not_in_scoped_process_environment() {
        let mut command = Command::new("test-command");
        command.env("INFIMOUNT_SECRET_MARKER", "must-never-cross-boundary");
        scoped_environment(&mut command);
        assert!(!command
            .get_envs()
            .any(|(key, value)| { key == "INFIMOUNT_SECRET_MARKER" && value.is_some() }));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_executable_substitution_before_spawn() {
        let temp = private_tempdir();
        let identity = mock_cli(temp.path(), "printf '%s' '{\"is_error\":false}'");
        fs::write(&identity.executable, b"changed").unwrap();
        assert!(execute_verified_claude(
            &identity,
            &[],
            temp.path(),
            &AtomicBool::new(false),
            Duration::from_secs(3)
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_owned_process_group() {
        let temp = private_tempdir();
        let identity = mock_cli(temp.path(), "sleep 10");
        let start = Instant::now();
        let outcome = execute_verified_claude(
            &identity,
            &[],
            temp.path(),
            &AtomicBool::new(false),
            Duration::from_millis(250),
        )
        .unwrap();
        assert_eq!(outcome.result, ClaudeOutcome::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_is_reported_without_launch() {
        let temp = private_tempdir();
        let identity = mock_cli(temp.path(), "exit 0");
        let cancelled = AtomicBool::new(true);
        let outcome = execute_verified_claude(
            &identity,
            &[],
            temp.path(),
            &cancelled,
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(outcome.result, ClaudeOutcome::Cancelled);
        assert_eq!(outcome.exit_code, None);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_json_or_cli_error_does_not_claim_success() {
        let temp = private_tempdir();
        for output in ["not-json", r#"{"is_error":true}"#] {
            let identity = mock_cli(temp.path(), &format!("printf '%s' '{output}'"));
            let outcome = execute_verified_claude(
                &identity,
                &[],
                temp.path(),
                &AtomicBool::new(false),
                Duration::from_secs(3),
            )
            .unwrap();
            assert_eq!(outcome.result, ClaudeOutcome::Failed);
        }
    }
}
