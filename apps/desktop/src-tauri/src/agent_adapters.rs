//! Agent Task adapters are *execution* targets, not storage authority.
//! Only Codex is enabled in A3. Other clients must prove a fail-closed
//! capability profile before their adapters can become available.
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use infimount_core::CoreError;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::client_integrations::run_with_timeout;

const CLIENT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PROBE_OUTPUT: usize = 4 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentAdapterKind {
    Codex,
    ClaudeCode,
    OpenCode,
    GeminiCli,
    Pi,
}

impl AgentAdapterKind {
    fn client_id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
            Self::GeminiCli => "gemini_cli",
            Self::Pi => "pi",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::GeminiCli => "Gemini CLI",
            Self::Pi => "Pi",
        }
    }
}

const ADAPTERS: [AgentAdapterKind; 5] = [
    AgentAdapterKind::Codex,
    AgentAdapterKind::ClaudeCode,
    AgentAdapterKind::OpenCode,
    AgentAdapterKind::GeminiCli,
    AgentAdapterKind::Pi,
];

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdapterStatus {
    Ready,
    NotInstalled,
    NotImplemented,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentAdapterCapability {
    pub client: &'static str,
    pub display_name: &'static str,
    pub status: AdapterStatus,
    pub enabled: bool,
    pub version: Option<String>,
    /// Non-authoritative display only. Never expose the executable host path.
    pub sha256_prefix: Option<String>,
    pub isolation_profile: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutableIdentity {
    /// Private to the Rust control plane. Never serializable into an agent tool.
    pub executable: PathBuf,
    pub sha256: String,
    pub reported_version: String,
}

pub(crate) struct AgentLaunchPlan {
    pub adapter: AgentAdapterKind,
    pub executable: ExecutableIdentity,
    pub args: Vec<String>,
}

impl AgentLaunchPlan {
    pub fn codex(executable: ExecutableIdentity, args: Vec<String>) -> Result<Self, CoreError> {
        // A3 does not change the golden Codex argument builder. An adapter is
        // prohibited from substituting an unrestricted host execution profile.
        let fixed_restrictions = [
            "features.code_mode=false",
            "features.code_mode_host=true",
            "features.shell_tool=false",
            "features.unified_exec=false",
            "features.browser_use=false",
            "features.plugins=false",
            "features.computer_use=false",
            "features.request_permissions_tool=false",
            "web_search=\"disabled\"",
        ];
        let required_mcp_count = args
            .iter()
            .filter(|value| {
                value.starts_with("mcp_servers.infimount_agent_task_")
                    && value.ends_with(".required=true")
            })
            .count();
        if args.first().map(String::as_str) != Some("exec")
            || !args
                .windows(2)
                .any(|a| a[0] == "--sandbox" && a[1] == "read-only")
            || !args.iter().any(|v| v == "--ephemeral")
            || !args.iter().any(|v| v == "--ignore-user-config")
            || !args.iter().any(|v| v == "--ignore-rules")
            || !fixed_restrictions
                .iter()
                .all(|restriction| args.iter().any(|v| v == restriction))
            || required_mcp_count != 1
        {
            return Err(CoreError::Config(
                "Agent Task launch profile is not sufficiently restricted".into(),
            ));
        }
        Ok(Self {
            adapter: AgentAdapterKind::Codex,
            executable,
            args,
        })
    }

    pub fn revalidate(&self) -> Result<(), CoreError> {
        if self.adapter != AgentAdapterKind::Codex {
            return Err(CoreError::Config("Agent Task client is not enabled".into()));
        }
        verify_executable_identity(&self.executable)
    }
}

fn probe_error(message: &'static str) -> CoreError {
    CoreError::Config(message.to_string())
}

fn executable_digest(executable: &Path) -> Result<String, CoreError> {
    let metadata = fs::metadata(executable)
        .map_err(|_| probe_error("Agent Task client executable is unavailable"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(probe_error(
            "Agent Task client executable is invalid or too large",
        ));
    }
    let mut file = File::open(executable)
        .map_err(|_| probe_error("Agent Task client executable cannot be opened"))?;
    let mut sha = Sha256::new();
    let mut block = [0_u8; 16 * 1024];
    loop {
        let n = file
            .read(&mut block)
            .map_err(|_| probe_error("Agent Task client executable cannot be read"))?;
        if n == 0 {
            break;
        }
        sha.update(&block[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}

pub(crate) fn verify_executable_identity(identity: &ExecutableIdentity) -> Result<(), CoreError> {
    let canonical = fs::canonicalize(&identity.executable).map_err(|_| {
        probe_error("Agent Task client executable disappeared; check adapters again")
    })?;
    if canonical != identity.executable || executable_digest(&canonical)? != identity.sha256 {
        return Err(probe_error(
            "Agent Task client executable changed; check adapters again",
        ));
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > 128
        || value.lines().count() != 1
        || !value.is_ascii()
        || value.bytes().any(|b| b.is_ascii_control())
        || !(value.starts_with("codex-cli ") || value.starts_with("codex "))
    {
        return None;
    }
    let mut parts = value.split_whitespace();
    let _client = parts.next()?;
    let version = parts.next()?;
    if parts.next().is_some()
        || version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    Some(value.to_string())
}

fn probe_version(path: &Path) -> (Option<std::process::ExitStatus>, String, String, bool) {
    #[cfg(windows)]
    {
        // PowerShell can launch npm-installed .cmd/.ps1 shims, whereas direct
        // CreateProcess of a .cmd/.ps1 is not a reliable executable probe.
        let literal = path.to_string_lossy().replace('\'', "''");
        let script = format!("& '{literal}' --version");
        let args = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            script,
        ];
        run_with_timeout(
            Path::new("powershell.exe"),
            &args,
            CLIENT_PROBE_TIMEOUT,
            MAX_PROBE_OUTPUT,
        )
    }
    #[cfg(not(windows))]
    {
        let args = vec!["--version".to_owned()];
        run_with_timeout(path, &args, CLIENT_PROBE_TIMEOUT, MAX_PROBE_OUTPUT)
    }
}

fn detect_codex_at(path: &Path) -> Result<ExecutableIdentity, CoreError> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| probe_error("Codex executable could not be resolved"))?;
    if !canonical.is_absolute() {
        return Err(probe_error(
            "Codex executable must resolve to an absolute path",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&canonical)
            .map_err(|_| probe_error("Codex executable is unavailable"))?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(probe_error("Codex executable lacks execute permission"));
        }
    }
    let sha256 = executable_digest(&canonical)?;
    let (status, stdout, _, timeout) = probe_version(&canonical);
    if timeout || !status.is_some_and(|s| s.success()) {
        return Err(probe_error("Codex version could not be verified"));
    }
    let version =
        parse_version(&stdout).ok_or_else(|| probe_error("Unsupported Codex version output"))?;
    let identity = ExecutableIdentity {
        executable: canonical,
        sha256,
        reported_version: version,
    };
    verify_executable_identity(&identity)?;
    Ok(identity)
}

fn resolve_codex_binary() -> Result<PathBuf, CoreError> {
    // Match the historical terminal handoff: users may install Codex through
    // a login shell whose PATH differs from the desktop application's PATH.
    #[cfg(unix)]
    {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let shell = Path::new(&shell);
        if !shell.is_absolute() {
            return Err(probe_error("A trusted absolute login shell is required"));
        }
        let args = vec!["-lic".to_string(), "command -v codex".to_string()];
        let (status, output, _, timeout) =
            run_with_timeout(shell, &args, CLIENT_PROBE_TIMEOUT, MAX_PROBE_OUTPUT);
        if timeout || !status.is_some_and(|s| s.success()) {
            return Err(probe_error("Codex was not found in the login shell"));
        }
        for line in output.lines().rev() {
            let candidate = Path::new(line.trim());
            if candidate.is_absolute() && candidate.is_file() {
                return Ok(candidate.to_path_buf());
            }
        }
        Err(probe_error(
            "Codex executable could not be resolved from the login shell",
        ))
    }
    #[cfg(windows)]
    {
        let script = "$ErrorActionPreference = 'Stop'; $c = Get-Command codex -ErrorAction Stop; if ($c.CommandType -notin @('Application', 'ExternalScript')) { exit 2 }; [Console]::WriteLine($c.Source)";
        let args = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ];
        let (status, output, _, timeout) = run_with_timeout(
            Path::new("powershell.exe"),
            &args,
            CLIENT_PROBE_TIMEOUT,
            MAX_PROBE_OUTPUT,
        );
        if timeout || !status.is_some_and(|s| s.success()) {
            return Err(probe_error("Codex was not found in PowerShell"));
        }
        let path = PathBuf::from(output.trim());
        if !path.is_absolute() || !path.is_file() {
            return Err(probe_error("Codex executable could not be resolved"));
        }
        Ok(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(probe_error(
            "Codex is not supported on this operating system",
        ))
    }
}

pub(crate) fn require_codex() -> Result<ExecutableIdentity, CoreError> {
    detect_codex_at(&resolve_codex_binary()?)
}

pub(crate) fn adapter_capabilities() -> Vec<AgentAdapterCapability> {
    ADAPTERS
        .iter()
        .copied()
        .map(|kind| {
            if kind == AgentAdapterKind::Codex {
                match require_codex() {
                    Ok(identity) => AgentAdapterCapability {
                        client: kind.client_id(),
                        display_name: kind.display_name(),
                        status: AdapterStatus::Ready,
                        enabled: true,
                        version: Some(identity.reported_version),
                        sha256_prefix: Some(identity.sha256[..12].to_string()),
                        isolation_profile: "mcp_only_readonly_host",
                        message: "Task-scoped MCP, read-only host sandbox, reviewed publication",
                    },
                    Err(_) => AgentAdapterCapability {
                        client: kind.client_id(),
                        display_name: kind.display_name(),
                        status: AdapterStatus::NotInstalled,
                        enabled: false,
                        version: None,
                        sha256_prefix: None,
                        isolation_profile: "mcp_only_readonly_host",
                        message: "Codex is unavailable or its executable/version is not verified",
                    },
                }
            } else {
                AgentAdapterCapability {
                    client: kind.client_id(),
                    display_name: kind.display_name(),
                    status: AdapterStatus::NotImplemented,
                    enabled: false,
                    version: None,
                    sha256_prefix: None,
                    isolation_profile: "mcp_only_readonly_host",
                    message: "First-class Agent Task handoff is not yet implemented",
                }
            }
        })
        .collect()
}

#[tauri::command]
pub async fn list_agent_task_adapters() -> Result<Vec<AgentAdapterCapability>, String> {
    tokio::task::spawn_blocking(adapter_capabilities)
        .await
        .map_err(|_| "Agent Task adapter detection could not complete".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_registry_never_enables_future_adapters() {
        let list = adapter_capabilities();
        assert_eq!(list.len(), 5);
        for entry in list.iter().skip(1) {
            assert!(!entry.enabled);
            assert!(matches!(entry.status, AdapterStatus::NotImplemented));
        }
    }

    #[test]
    fn untrusted_version_lines_are_rejected() {
        assert!(parse_version("codex-cli 0.155.1").is_some());
        for text in [
            "",
            "Codex unknown",
            "codex 0.1\nsecret",
            "codex ../secrets",
            "codex 0.1 secret",
            "Bearer secret",
            "codex \u{1b}[31m",
        ] {
            assert!(parse_version(text).is_none(), "{text:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn executable_digest_rejects_substitution_and_inactive_binary() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex");
        fs::write(&path, "#!/bin/sh\necho 'codex-cli 0.155.1'\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let identity = detect_codex_at(&path).unwrap();
        assert_eq!(identity.reported_version, "codex-cli 0.155.1");
        verify_executable_identity(&identity).unwrap();
        fs::write(&path, "#!/bin/sh\necho 'codex-cli 0.155.2'\n").unwrap();
        assert!(verify_executable_identity(&identity).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(detect_codex_at(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn codex_launch_plan_rejects_modified_executable_after_preview() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("codex");
        fs::write(&binary, "#!/bin/sh\necho 'codex-cli 0.155.1'\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let identity = detect_codex_at(&binary).unwrap();
        let mut args = vec![
            "exec".into(),
            "--sandbox".into(),
            "read-only".into(),
            "--ephemeral".into(),
            "--ignore-user-config".into(),
            "--ignore-rules".into(),
        ];
        for flag in [
            "features.code_mode=false",
            "features.code_mode_host=true",
            "features.shell_tool=false",
            "features.unified_exec=false",
            "features.browser_use=false",
            "features.plugins=false",
            "features.computer_use=false",
            "features.request_permissions_tool=false",
            "web_search=\"disabled\"",
            "mcp_servers.infimount_agent_task_fixture.required=true",
        ] {
            args.push(flag.into());
        }
        let plan = AgentLaunchPlan::codex(identity, args).unwrap();
        plan.revalidate().unwrap();
        fs::write(&binary, "#!/bin/sh\necho 'codex-cli 0.155.2'\n").unwrap();
        assert!(plan.revalidate().is_err());
    }

    #[test]
    fn codex_launch_plan_refuses_incomplete_sandbox_profile() {
        let identity = ExecutableIdentity {
            executable: PathBuf::from("/abs/not-a-real-client"),
            sha256: "a".repeat(64),
            reported_version: "codex-cli 0.155.1".into(),
        };
        let args = vec!["exec".into(), "--sandbox".into(), "read-only".into()];
        assert!(AgentLaunchPlan::codex(identity, args).is_err());
    }
}
