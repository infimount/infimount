use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, ExitStatus, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);
const CLIENT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CAPTURED_OUTPUT: usize = 8 * 1024;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPORTED_VERSION_LEN: usize = 160;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpClientKind {
    GenericStdio,
    ClaudeCode,
    Cursor,
    VsCode,
    OpenCode,
    ClaudeDesktop,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientAdapterInfo {
    pub kind: McpClientKind,
    pub name: &'static str,
    pub description: &'static str,
    pub detected: bool,
    pub detection: String,
    pub write_capable: bool,
    pub requires_execution_confirmation: bool,
    pub default_target: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInstallInput {
    pub kind: McpClientKind,
    #[serde(default)]
    pub target_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInstallPreview {
    pub preview_id: String,
    pub kind: McpClientKind,
    pub action: String,
    pub target_path: Option<String>,
    pub before: Option<String>,
    pub after: String,
    pub can_apply: bool,
    pub requires_execution_confirmation: bool,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyClientInstallInput {
    pub preview_id: String,
    #[serde(default)]
    pub confirm_execution: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInstallResult {
    pub applied: bool,
    pub target_path: Option<String>,
    pub backup_path: Option<String>,
    pub rollback_id: Option<String>,
}

#[derive(Clone)]
enum InstallAction {
    CopyOnly,
    File {
        target: PathBuf,
        before: Option<Vec<u8>>,
        after: Vec<u8>,
    },
    Command(ClientCommand),
}

#[derive(Clone)]
struct ClientCommand {
    name: String,
    executable: PathBuf,
    executable_digest: String,
    #[allow(dead_code)]
    reported_version: String,
    target_digest: String,
    args: Vec<String>,
    sidecar: PathBuf,
    rollback_target: PathBuf,
    before: Option<Vec<u8>>,
}

#[derive(Clone)]
struct StoredPreview {
    created_at: Instant,
    kind: McpClientKind,
    action: InstallAction,
}

struct RollbackRecord {
    created_at: Instant,
    target: PathBuf,
    before: Option<Vec<u8>>,
    applied_digest: String,
}

static PREVIEWS: OnceLock<Mutex<HashMap<String, StoredPreview>>> = OnceLock::new();
static ROLLBACKS: OnceLock<Mutex<HashMap<String, RollbackRecord>>> = OnceLock::new();
#[cfg(not(test))]
static CLIENT_EVENTS: OnceLock<infimount_mcp::telemetry::ProductEventStore> = OnceLock::new();

fn previews() -> &'static Mutex<HashMap<String, StoredPreview>> {
    PREVIEWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rollbacks() -> &'static Mutex<HashMap<String, RollbackRecord>> {
    ROLLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_error() -> String {
    "client integration state is unavailable".to_string()
}

#[cfg(not(test))]
fn record_client_event(kind: McpClientKind, name: infimount_mcp::telemetry::ProductEventName) {
    let mut event = infimount_mcp::telemetry::ProductEvent::new(name);
    event.client_kind = Some(
        serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "other".to_string()),
    );
    event.success = Some(true);
    let store =
        CLIENT_EVENTS.get_or_init(|| infimount_mcp::telemetry::ProductEventStore::new(None));
    let _ = store.record(event);
}

#[cfg(test)]
fn record_client_event(_kind: McpClientKind, _name: infimount_mcp::telemetry::ProductEventName) {}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn redact_display_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
                if normalized.contains("token")
                    || normalized.contains("password")
                    || normalized.contains("secret")
                    || normalized.contains("authorization")
                    || normalized == "apikey"
                    || normalized == "headers"
                {
                    *child = Value::String("<redacted>".into());
                } else if normalized == "url" {
                    if let Some(url) = child.as_str() {
                        *child = Value::String(url.split('?').next().unwrap_or(url).to_string());
                    }
                } else {
                    redact_display_value(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_display_value),
        _ => {}
    }
}

fn display_config(bytes: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return "<existing config hidden because it is not valid JSON>".to_string();
    };
    redact_display_value(&mut value);
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "<config unavailable>".into())
}

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".config")))
}

fn command_config_target(kind: McpClientKind) -> Option<PathBuf> {
    match kind {
        McpClientKind::ClaudeCode => home_dir().map(|home| home.join(".claude.json")),
        McpClientKind::VsCode => {
            #[cfg(target_os = "macos")]
            {
                home_dir().map(|home| home.join("Library/Application Support/Code/User/mcp.json"))
            }
            #[cfg(target_os = "windows")]
            {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .map(|dir| dir.join("Code/User/mcp.json"))
            }
            #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
            {
                config_home().map(|dir| dir.join("Code/User/mcp.json"))
            }
        }
        _ => None,
    }
}

fn default_target(kind: McpClientKind) -> Option<PathBuf> {
    match kind {
        McpClientKind::Cursor => home_dir().map(|home| home.join(".cursor/mcp.json")),
        McpClientKind::VsCode => std::env::current_dir()
            .ok()
            .map(|dir| dir.join(".vscode/mcp.json")),
        McpClientKind::OpenCode => config_home().map(|dir| dir.join("opencode/opencode.json")),
        McpClientKind::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home_dir().map(|home| {
                    home.join("Library/Application Support/Claude/claude_desktop_config.json")
                })
            }
            #[cfg(target_os = "windows")]
            {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .map(|dir| dir.join("Claude/claude_desktop_config.json"))
            }
            #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
            {
                config_home().map(|dir| dir.join("Claude/claude_desktop_config.json"))
            }
        }
        McpClientKind::GenericStdio | McpClientKind::ClaudeCode => None,
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    std::env::split_paths(&path)
        .map(|dir| dir.join(format!("{name}{suffix}")))
        .find(|candidate| is_executable(candidate))
}

fn capture_stdout(pipe: impl Read, cap: usize) -> String {
    let mut buf = Vec::with_capacity(cap.min(64 * 1024));
    let mut chunk = [0_u8; 4096];
    let mut handle = pipe;
    loop {
        match handle.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < cap {
                    let keep = n.min(cap - buf.len());
                    buf.extend_from_slice(&chunk[..keep]);
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn run_with_timeout(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    max_output: usize,
) -> (Option<ExitStatus>, String, String, bool) {
    let mut command = StdCommand::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return (None, String::new(), String::new(), false),
    };
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stdout_handle = thread::spawn(move || capture_stdout(stdout, max_output));
    let stderr_handle = thread::spawn(move || capture_stdout(stderr, max_output));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if Instant::now() >= deadline {
            kill_process_tree(&mut child);
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return (None, String::new(), String::new(), true);
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    (status, stdout, stderr, false)
}

fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe {
            let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let _ = windows_sys::Win32::System::JobObjects::SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                let process_handle: HANDLE =
                    OpenProcess(PROCESS_TERMINATE | PROCESS_SET_QUOTA, 0, child.id());
                if !process_handle.is_null() {
                    let _ = AssignProcessToJobObject(job, process_handle);
                    CloseHandle(process_handle);
                }
                // Closing the job handle triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                // terminating all processes in the job (parent + descendants).
                CloseHandle(job);
            }
        }
        // Fallback: also kill the direct child in case Job Object creation failed.
        let _ = child.kill();
    }
    #[cfg(not(unix))]
    #[cfg(not(target_os = "windows"))]
    {
        let _ = child.kill();
    }
}

fn command_identity(
    executable: &Path,
    name: &str,
    version_args: &[String],
) -> Option<ClientCommandBase> {
    let canonical = fs::canonicalize(executable).ok()?;
    let bytes = fs::read(&canonical).ok()?;
    let (status, stdout, _, timed_out) =
        run_with_timeout(&canonical, version_args, VERSION_PROBE_TIMEOUT, 8 * 1024);
    // Reject probes that timed out, failed to execute, or exited unsuccessfully.
    if timed_out || status.is_none() || !status.unwrap().success() {
        return None;
    }
    let reported_version = stdout
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_REPORTED_VERSION_LEN)
        .collect::<String>();
    let reported_version = if reported_version.is_empty() {
        "unknown".to_string()
    } else {
        reported_version
    };
    Some(ClientCommandBase {
        name: name.to_string(),
        executable: canonical,
        executable_digest: digest(&bytes),
        reported_version,
    })
}

struct ClientCommandBase {
    name: String,
    executable: PathBuf,
    executable_digest: String,
    reported_version: String,
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn sidecar_config(path: &Path) -> Value {
    json!({
        "command": path.to_string_lossy(),
        "args": ["serve", "--transport", "stdio"]
    })
}

fn generic_snippet(path: &Path) -> String {
    serde_json::to_string_pretty(&json!({
        "mcpServers": { "infimount": sidecar_config(path) }
    }))
    .unwrap_or_default()
}

fn claude_code_command(path: &Path) -> String {
    let encoded = serde_json::to_string(&path.to_string_lossy()).unwrap_or_else(|_| "\"\"".into());
    format!("claude mcp add infimount -- {encoded} serve --transport stdio")
}

fn vscode_cli_payload(path: &Path) -> String {
    serde_json::to_string(&json!({
        "name": "infimount",
        "command": path.to_string_lossy(),
        "args": ["serve", "--transport", "stdio"]
    }))
    .unwrap_or_default()
}

fn adapter_info(kind: McpClientKind, sidecar: &Path) -> ClientAdapterInfo {
    let target = default_target(kind);
    let (name, description, detected, detection, write_capable, confirm, snippet) = match kind {
        McpClientKind::GenericStdio => (
            "Generic stdio JSON",
            "Copy a standard MCP stdio server entry.",
            true,
            "Always available (copy only)".to_string(),
            false,
            false,
            generic_snippet(sidecar),
        ),
        McpClientKind::ClaudeCode => {
            let cli = find_executable("claude");
            (
                "Claude Code",
                "Preview a verified claude mcp add command.",
                cli.is_some(),
                cli.as_ref().map_or_else(
                    || "Claude CLI not found".into(),
                    |p| p.display().to_string(),
                ),
                cli.is_some(),
                true,
                claude_code_command(sidecar),
            )
        }
        McpClientKind::Cursor => (
            "Cursor",
            "Merge only the Infimount entry into Cursor MCP JSON.",
            target.as_ref().is_some_and(|path| path.exists()),
            target.as_ref().map_or_else(
                || "Cursor config path unavailable".into(),
                |p| p.display().to_string(),
            ),
            true,
            false,
            generic_snippet(sidecar),
        ),
        McpClientKind::VsCode => {
            let cli = find_executable("code");
            (
                "VS Code",
                "Use code --add-mcp when available, otherwise merge project MCP JSON.",
                cli.is_some() || target.as_ref().is_some_and(|path| path.exists()),
                cli.as_ref().map_or_else(
                    || {
                        target.as_ref().map_or_else(
                            || "VS Code target unavailable".into(),
                            |p| p.display().to_string(),
                        )
                    },
                    |p| p.display().to_string(),
                ),
                true,
                cli.is_some(),
                generic_snippet(sidecar),
            )
        }
        McpClientKind::OpenCode => (
            "OpenCode",
            "Merge Infimount into a plain JSON OpenCode config.",
            target.as_ref().is_some_and(|path| path.exists()),
            target.as_ref().map_or_else(
                || "OpenCode config path unavailable".into(),
                |p| p.display().to_string(),
            ),
            true,
            false,
            serde_json::to_string_pretty(&json!({
                "mcp": { "infimount": {
                    "type": "local",
                    "command": [sidecar.to_string_lossy(), "serve", "--transport", "stdio"],
                    "enabled": true,
                    "timeout": 10000
                }}
            }))
            .unwrap_or_default(),
        ),
        McpClientKind::ClaudeDesktop => (
            "Claude Desktop",
            "Copy JSON for the detected Claude Desktop config location.",
            target.as_ref().is_some_and(|path| path.exists()),
            target.as_ref().map_or_else(
                || "Claude Desktop config path unavailable".into(),
                |p| p.display().to_string(),
            ),
            false,
            false,
            generic_snippet(sidecar),
        ),
    };
    ClientAdapterInfo {
        kind,
        name,
        description,
        detected,
        detection,
        write_capable,
        requires_execution_confirmation: confirm,
        default_target: if kind == McpClientKind::VsCode && find_executable("code").is_some() {
            None
        } else {
            target.map(|path| path.to_string_lossy().to_string())
        },
        snippet,
    }
}

#[tauri::command]
pub fn list_mcp_client_adapters() -> Result<Vec<ClientAdapterInfo>, String> {
    let sidecar = crate::activation_probe::verified_sidecar_path()
        .map_err(|code| format!("bundled MCP sidecar is unavailable ({code})"))?;
    Ok([
        McpClientKind::GenericStdio,
        McpClientKind::ClaudeCode,
        McpClientKind::Cursor,
        McpClientKind::VsCode,
        McpClientKind::OpenCode,
        McpClientKind::ClaudeDesktop,
    ]
    .into_iter()
    .map(|kind| adapter_info(kind, &sidecar))
    .collect())
}

fn validate_target(kind: McpClientKind, requested: Option<String>) -> Result<PathBuf, String> {
    let target = requested
        .map(PathBuf::from)
        .or_else(|| default_target(kind))
        .ok_or_else(|| "client config target is unavailable".to_string())?;
    if !target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("client config target must be an absolute path without '..'".to_string());
    }
    let normalized = target.to_string_lossy().replace('\\', "/");
    let valid = match kind {
        McpClientKind::Cursor => normalized.ends_with("/.cursor/mcp.json"),
        McpClientKind::VsCode => normalized.ends_with("/.vscode/mcp.json"),
        McpClientKind::OpenCode => {
            normalized.ends_with("/opencode/opencode.json")
                || normalized.ends_with("/opencode.json")
        }
        _ => false,
    };
    if !valid {
        return Err("client config target does not match the selected adapter".to_string());
    }
    Ok(target)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if path.exists() {
        fs::read(path)
            .map(Some)
            .map_err(|_| "failed to read client config".to_string())
    } else {
        Ok(None)
    }
}

type LoadedJsonObject = (Option<Vec<u8>>, Map<String, Value>);

fn load_json_object(path: &Path) -> Result<LoadedJsonObject, String> {
    if !path.exists() {
        return Ok((None, Map::new()));
    }
    let bytes = fs::read(path).map_err(|_| "failed to read client config".to_string())?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "client config must be valid plain JSON without comments".to_string())?;
    let object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "client config root must be a JSON object".to_string())?;
    Ok((Some(bytes), object))
}

fn merge_client_config(
    kind: McpClientKind,
    target: &Path,
    sidecar: &Path,
) -> Result<(Option<Vec<u8>>, Vec<u8>), String> {
    let (before, mut root) = load_json_object(target)?;
    match kind {
        McpClientKind::Cursor => {
            let servers = root
                .entry("mcpServers")
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| "mcpServers must be a JSON object".to_string())?;
            servers.insert("infimount".into(), sidecar_config(sidecar));
        }
        McpClientKind::VsCode => {
            let servers = root
                .entry("servers")
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| "servers must be a JSON object".to_string())?;
            let mut config = sidecar_config(sidecar);
            config
                .as_object_mut()
                .expect("object")
                .insert("type".into(), json!("stdio"));
            servers.insert("infimount".into(), config);
        }
        McpClientKind::OpenCode => {
            let mcp = root
                .entry("mcp")
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| "mcp must be a JSON object".to_string())?;
            mcp.insert(
                "infimount".into(),
                json!({
                    "type": "local",
                    "command": [sidecar.to_string_lossy(), "serve", "--transport", "stdio"],
                    "enabled": true,
                    "timeout": 10000
                }),
            );
        }
        _ => return Err("selected adapter does not support config writes".to_string()),
    }
    let mut after = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|_| "failed to serialize client config".to_string())?;
    after.push(b'\n');
    Ok((before, after))
}

#[tauri::command]
pub fn preview_mcp_client_install(
    input: ClientInstallInput,
) -> Result<ClientInstallPreview, String> {
    let sidecar = crate::activation_probe::verified_sidecar_path()
        .map_err(|code| format!("bundled MCP sidecar is unavailable ({code})"))?;
    let (action_name, target_path, before_display, after, can_apply, confirm, action) =
        match input.kind {
            McpClientKind::GenericStdio | McpClientKind::ClaudeDesktop => {
                let snippet = generic_snippet(&sidecar);
                (
                    "copy",
                    default_target(input.kind),
                    None,
                    snippet,
                    false,
                    false,
                    InstallAction::CopyOnly,
                )
            }
            McpClientKind::ClaudeCode => {
                let executable = find_executable("claude")
                    .ok_or_else(|| "Claude CLI was not detected".to_string())?;
                let command = claude_code_command(&sidecar);
                let rollback_target = command_config_target(McpClientKind::ClaudeCode)
                    .ok_or_else(|| "Claude Code config path is unavailable".to_string())?;
                let before = read_optional(&rollback_target)?;
                let before_display = before.as_ref().map(|bytes| display_config(bytes));
                let base = command_identity(&executable, "claude", &["--version".to_string()])
                    .ok_or_else(|| "Claude CLI could not be verified".to_string())?;
                (
                    "execute",
                    Some(rollback_target.clone()),
                    before_display,
                    command,
                    true,
                    true,
                    InstallAction::Command(ClientCommand {
                        name: base.name,
                        executable: base.executable,
                        executable_digest: base.executable_digest,
                        reported_version: base.reported_version,
                        target_digest: digest(before.as_deref().unwrap_or_default()),
                        sidecar: sidecar.clone(),
                        rollback_target,
                        before,
                        args: vec![
                            "mcp".into(),
                            "add".into(),
                            "infimount".into(),
                            "--".into(),
                            sidecar.to_string_lossy().to_string(),
                            "serve".into(),
                            "--transport".into(),
                            "stdio".into(),
                        ],
                    }),
                )
            }
            McpClientKind::VsCode
                if find_executable("code").is_some() && input.target_path.is_none() =>
            {
                let executable = find_executable("code").expect("checked");
                let snippet = vscode_cli_payload(&sidecar);
                let rollback_target = command_config_target(McpClientKind::VsCode)
                    .ok_or_else(|| "VS Code user MCP config path is unavailable".to_string())?;
                let before = read_optional(&rollback_target)?;
                let before_display = before.as_ref().map(|bytes| display_config(bytes));
                let base = command_identity(&executable, "code", &["--version".to_string()])
                    .ok_or_else(|| "VS Code CLI could not be verified".to_string())?;
                (
                    "execute",
                    Some(rollback_target.clone()),
                    before_display,
                    format!("code --add-mcp {snippet}"),
                    true,
                    true,
                    InstallAction::Command(ClientCommand {
                        name: base.name,
                        executable: base.executable,
                        executable_digest: base.executable_digest,
                        reported_version: base.reported_version,
                        target_digest: digest(before.as_deref().unwrap_or_default()),
                        sidecar: sidecar.clone(),
                        rollback_target,
                        before,
                        args: vec!["--add-mcp".into(), snippet],
                    }),
                )
            }
            McpClientKind::Cursor | McpClientKind::VsCode | McpClientKind::OpenCode => {
                let target = validate_target(input.kind, input.target_path)?;
                let (before, after) = merge_client_config(input.kind, &target, &sidecar)?;
                let before_display = before.as_ref().map(|bytes| display_config(bytes));
                let after_display = display_config(&after);
                (
                    "write",
                    Some(target.clone()),
                    before_display,
                    after_display,
                    true,
                    false,
                    InstallAction::File {
                        target,
                        before,
                        after,
                    },
                )
            }
        };
    let preview_id = Uuid::new_v4().to_string();
    let mut pending = previews().lock().map_err(|_| lock_error())?;
    pending.retain(|_, item| item.created_at.elapsed() <= PREVIEW_TTL);
    if pending.len() >= 128 {
        if let Some(oldest) = pending
            .iter()
            .min_by_key(|(_, item)| item.created_at)
            .map(|(id, _)| id.clone())
        {
            pending.remove(&oldest);
        }
    }
    pending.insert(
        preview_id.clone(),
        StoredPreview {
            created_at: Instant::now(),
            kind: input.kind,
            action,
        },
    );
    drop(pending);
    record_client_event(
        input.kind,
        infimount_mcp::telemetry::ProductEventName::ClientConfigPreviewed,
    );
    Ok(ClientInstallPreview {
        preview_id,
        kind: input.kind,
        action: action_name.into(),
        target_path: target_path.map(|path| path.to_string_lossy().to_string()),
        before: before_display,
        after,
        can_apply,
        requires_execution_confirmation: confirm,
        expires_in_seconds: PREVIEW_TTL.as_secs(),
    })
}

fn write_config_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "client config has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "failed to create client config directory".to_string())?;
    infimount_core::atomic_file::atomic_write_file(
        path,
        bytes,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| "failed to write client config".to_string())
}

fn restore_client_config(target: &Path, before: Option<&[u8]>) -> Result<(), String> {
    match before {
        Some(bytes) => write_config_atomic(target, bytes),
        None if target.exists() => fs::remove_file(target)
            .map_err(|_| "failed to roll back client command config".to_string()),
        None => Ok(()),
    }
}

fn backup_path(target: &Path) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!("{name}.backup-{timestamp}-{}", Uuid::new_v4()))
}

fn store_rollback(
    target: PathBuf,
    before: Option<Vec<u8>>,
    after: &[u8],
) -> Result<String, String> {
    let rollback_id = Uuid::new_v4().to_string();
    let mut pending = rollbacks().lock().map_err(|_| lock_error())?;
    pending.retain(|_, item| item.created_at.elapsed() <= PREVIEW_TTL);
    if pending.len() >= 128 {
        if let Some(oldest) = pending
            .iter()
            .min_by_key(|(_, item)| item.created_at)
            .map(|(id, _)| id.clone())
        {
            pending.remove(&oldest);
        }
    }
    pending.insert(
        rollback_id.clone(),
        RollbackRecord {
            created_at: Instant::now(),
            target,
            before,
            applied_digest: digest(after),
        },
    );
    Ok(rollback_id)
}

#[tauri::command]
pub fn apply_mcp_client_install(
    input: ApplyClientInstallInput,
) -> Result<ClientInstallResult, String> {
    let preview = previews()
        .lock()
        .map_err(|_| lock_error())?
        .remove(&input.preview_id)
        .ok_or_else(|| "client install preview was not found or was already used".to_string())?;
    if preview.created_at.elapsed() > PREVIEW_TTL {
        return Err("client install preview expired".to_string());
    }
    match preview.action {
        InstallAction::CopyOnly => {
            Err("copy-only adapters cannot be applied automatically".to_string())
        }
        InstallAction::Command(command) => {
            if !input.confirm_execution {
                return Err(
                    "explicit confirmation is required before executing a client command"
                        .to_string(),
                );
            }
            let result = apply_client_command(
                command,
                &crate::activation_probe::verified_sidecar_path()
                    .map_err(|code| format!("bundled MCP sidecar is unavailable ({code})"))?,
                input.confirm_execution,
            )?;
            record_client_event(
                preview.kind,
                infimount_mcp::telemetry::ProductEventName::ClientConfigApplied,
            );
            Ok(result)
        }
        InstallAction::File {
            target,
            before,
            after,
        } => {
            let current = if target.exists() {
                Some(fs::read(&target).map_err(|_| "failed to re-read client config".to_string())?)
            } else {
                None
            };
            if current != before {
                return Err("client config changed after preview; preview again".to_string());
            }
            let backup = if let Some(bytes) = before.as_ref() {
                let path = backup_path(&target);
                write_config_atomic(&path, bytes)?;
                Some(path)
            } else {
                None
            };
            write_config_atomic(&target, &after)?;
            let rollback_id = store_rollback(target.clone(), before, &after)?;
            record_client_event(
                preview.kind,
                infimount_mcp::telemetry::ProductEventName::ClientConfigApplied,
            );
            Ok(ClientInstallResult {
                applied: true,
                target_path: Some(target.to_string_lossy().to_string()),
                backup_path: backup.map(|path| path.to_string_lossy().to_string()),
                rollback_id: Some(rollback_id),
            })
        }
    }
}

fn apply_client_command(
    command: ClientCommand,
    verified_sidecar: &Path,
    confirmed: bool,
) -> Result<ClientInstallResult, String> {
    if !confirmed {
        return Err(
            "explicit confirmation is required before executing a client command".to_string(),
        );
    }
    if verified_sidecar != command.sidecar {
        return Err("bundled MCP sidecar changed after preview; preview again".to_string());
    }
    let current = read_optional(&command.rollback_target)?;
    if digest(current.as_deref().unwrap_or_default()) != command.target_digest {
        return Err("client config changed after preview; preview again".to_string());
    }
    let rechecked = fs::canonicalize(&command.executable)
        .map_err(|_| "client executable is no longer accessible; preview again".to_string())?;
    if rechecked != command.executable {
        return Err(format!(
            "{} executable changed after preview; preview again",
            command.name
        ));
    }
    let bytes = fs::read(&command.executable)
        .map_err(|_| "client executable could not be re-read; preview again".to_string())?;
    if digest(&bytes) != command.executable_digest {
        return Err(format!(
            "{} executable changed after preview; preview again",
            command.name
        ));
    }
    let backup = if let Some(bytes) = command.before.as_ref() {
        let path = backup_path(&command.rollback_target);
        write_config_atomic(&path, bytes)?;
        Some(path)
    } else {
        None
    };
    let (status, _, _, timed_out) = run_with_timeout(
        &command.executable,
        &command.args,
        CLIENT_COMMAND_TIMEOUT,
        MAX_CAPTURED_OUTPUT,
    );
    if timed_out {
        restore_client_config(&command.rollback_target, command.before.as_deref())?;
        return Err("client command timed out and the config change was rolled back".to_string());
    }
    let status = match status {
        Some(status) => status,
        None => {
            restore_client_config(&command.rollback_target, command.before.as_deref())?;
            return Err("failed to execute client command; config was rolled back".to_string());
        }
    };
    if !status.success() {
        restore_client_config(&command.rollback_target, command.before.as_deref())?;
        return Err("client command failed; config was rolled back".to_string());
    }
    let after = match fs::read(&command.rollback_target) {
        Ok(after) => after,
        Err(_) => {
            restore_client_config(&command.rollback_target, command.before.as_deref())?;
            return Err(
                "client command change could not be verified and was rolled back".to_string(),
            );
        }
    };
    let sidecar_text = command.sidecar.to_string_lossy();
    if !String::from_utf8_lossy(&after).contains(sidecar_text.as_ref()) {
        restore_client_config(&command.rollback_target, command.before.as_deref())?;
        return Err(
            "client command did not install the verified sidecar and was rolled back".to_string(),
        );
    }
    let rollback_id = store_rollback(command.rollback_target.clone(), command.before, &after)?;
    Ok(ClientInstallResult {
        applied: true,
        target_path: Some(command.rollback_target.to_string_lossy().to_string()),
        backup_path: backup.map(|path| path.to_string_lossy().to_string()),
        rollback_id: Some(rollback_id),
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub fn rollback_mcp_client_install(rollbackId: String) -> Result<(), String> {
    let rollback = rollbacks()
        .lock()
        .map_err(|_| lock_error())?
        .remove(&rollbackId)
        .ok_or_else(|| "client install rollback was not found or was already used".to_string())?;
    if rollback.created_at.elapsed() > PREVIEW_TTL {
        return Err("client install rollback expired".to_string());
    }
    let current = fs::read(&rollback.target)
        .map_err(|_| "failed to read installed client config".to_string())?;
    if digest(&current) != rollback.applied_digest {
        return Err("client config changed after install; refusing rollback".to_string());
    }
    match rollback.before {
        Some(bytes) => write_config_atomic(&rollback.target, &bytes),
        None => fs::remove_file(&rollback.target)
            .map_err(|_| "failed to remove installed client config".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fake_sidecar() -> PathBuf {
        PathBuf::from("/verified/Infimount Sidecar/mcp")
    }

    #[test]
    fn cursor_merge_preserves_unrelated_servers_and_creates_backup_and_rollback() {
        let dir = tempdir().unwrap();
        let target = dir.path().join(".cursor/mcp.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = br#"{"mcpServers":{"other":{"command":"other"}},"setting":true}"#;
        fs::write(&target, original).unwrap();
        let (before, after) =
            merge_client_config(McpClientKind::Cursor, &target, &fake_sidecar()).unwrap();
        let parsed: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(parsed["setting"], true);
        assert_eq!(parsed["mcpServers"]["infimount"]["args"][0], "serve");

        let preview_id = Uuid::new_v4().to_string();
        previews().lock().unwrap().insert(
            preview_id.clone(),
            StoredPreview {
                created_at: Instant::now(),
                kind: McpClientKind::Cursor,
                action: InstallAction::File {
                    target: target.clone(),
                    before,
                    after,
                },
            },
        );
        let result = apply_mcp_client_install(ApplyClientInstallInput {
            preview_id,
            confirm_execution: false,
        })
        .unwrap();
        assert!(Path::new(result.backup_path.as_deref().unwrap()).is_file());
        assert_ne!(fs::read(&target).unwrap(), original);
        rollback_mcp_client_install(result.rollback_id.unwrap()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), original);
    }

    #[test]
    fn all_adapters_generate_absolute_verified_sidecar_snippets_without_path_fallback() {
        let sidecar = fake_sidecar();
        for kind in [
            McpClientKind::GenericStdio,
            McpClientKind::ClaudeCode,
            McpClientKind::Cursor,
            McpClientKind::VsCode,
            McpClientKind::OpenCode,
            McpClientKind::ClaudeDesktop,
        ] {
            let info = adapter_info(kind, &sidecar);
            assert!(info.snippet.contains("/verified/Infimount Sidecar/mcp"));
            assert!(info.snippet.contains("serve"));
            assert!(!info.snippet.contains("\"command\": \"infimount_mcp\""));
        }
        assert!(default_target(McpClientKind::ClaudeDesktop).is_some());
    }

    #[test]
    fn preview_display_redacts_unrelated_client_secrets() {
        let display = display_config(
            br#"{"mcpServers":{"other":{"headers":{"Authorization":"Bearer raw"},"url":"https://example.test/mcp?signature=raw"}},"apiToken":"raw"}"#,
        );
        assert!(!display.contains("Bearer raw"));
        assert!(!display.contains("signature=raw"));
        assert!(!display.contains("\"raw\""));
        assert!(display.contains("<redacted>"));
    }

    #[test]
    fn malformed_and_commented_opencode_configs_are_refused() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("opencode/opencode.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"{ // comment\n \"mcp\": {} }").unwrap();
        assert!(merge_client_config(McpClientKind::OpenCode, &target, &fake_sidecar()).is_err());
        fs::write(&target, b"[]").unwrap();
        assert!(merge_client_config(McpClientKind::OpenCode, &target, &fake_sidecar()).is_err());
    }

    #[test]
    fn vscode_and_opencode_merges_use_verified_serve_command() {
        let dir = tempdir().unwrap();
        let vscode = dir.path().join(".vscode/mcp.json");
        let (_, after) =
            merge_client_config(McpClientKind::VsCode, &vscode, &fake_sidecar()).unwrap();
        let value: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(
            value["servers"]["infimount"]["command"],
            "/verified/Infimount Sidecar/mcp"
        );
        assert_eq!(
            value["servers"]["infimount"]["args"],
            json!(["serve", "--transport", "stdio"])
        );

        let opencode = dir.path().join("opencode/opencode.json");
        let (_, after) =
            merge_client_config(McpClientKind::OpenCode, &opencode, &fake_sidecar()).unwrap();
        let value: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(value["mcp"]["infimount"]["command"][1], "serve");
    }

    #[test]
    fn apply_refuses_stale_preview_and_command_without_confirmation() {
        let dir = tempdir().unwrap();
        let target = dir.path().join(".cursor/mcp.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"{}").unwrap();
        let (before, after) =
            merge_client_config(McpClientKind::Cursor, &target, &fake_sidecar()).unwrap();
        fs::write(&target, b"{\"changed\":true}").unwrap();
        let id = Uuid::new_v4().to_string();
        previews().lock().unwrap().insert(
            id.clone(),
            StoredPreview {
                created_at: Instant::now(),
                kind: McpClientKind::Cursor,
                action: InstallAction::File {
                    target,
                    before,
                    after,
                },
            },
        );
        assert!(apply_mcp_client_install(ApplyClientInstallInput {
            preview_id: id,
            confirm_execution: false
        })
        .is_err());

        let id = Uuid::new_v4().to_string();
        previews().lock().unwrap().insert(
            id.clone(),
            StoredPreview {
                created_at: Instant::now(),
                kind: McpClientKind::ClaudeCode,
                action: InstallAction::Command(ClientCommand {
                    name: "claude".to_string(),
                    executable: PathBuf::from("ignored"),
                    executable_digest: String::new(),
                    reported_version: "unknown".to_string(),
                    target_digest: String::new(),
                    args: vec![],
                    sidecar: fake_sidecar(),
                    rollback_target: dir.path().join(".claude.json"),
                    before: None,
                }),
            },
        );
        let error = apply_mcp_client_install(ApplyClientInstallInput {
            preview_id: id,
            confirm_execution: false,
        })
        .unwrap_err();
        assert!(error.contains("explicit confirmation"));
    }

    #[cfg(unix)]
    fn write_executable_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn test_command(
        script: &Path,
        args: Vec<String>,
        rollback_target: PathBuf,
        before: Option<Vec<u8>>,
    ) -> ClientCommand {
        let canonical = fs::canonicalize(script).unwrap();
        let bytes = fs::read(&canonical).unwrap();
        ClientCommand {
            name: "fake-cli".to_string(),
            executable: canonical,
            executable_digest: digest(&bytes),
            reported_version: "test".to_string(),
            target_digest: digest(before.as_deref().unwrap_or_default()),
            args,
            sidecar: fake_sidecar(),
            rollback_target,
            before,
        }
    }

    #[cfg(unix)]
    #[test]
    fn client_cli_replaced_after_preview_is_rejected() {
        let dir = tempdir().unwrap();
        let script = write_executable_script(dir.path(), "fake-cli", "exit 0");
        let target = dir.path().join(".claude.json");
        let before = b"{}".to_vec();
        fs::write(&target, &before).unwrap();
        let command = test_command(&script, vec![], target.clone(), Some(before));
        write_executable_script(dir.path(), "fake-cli", "exit 0 # replaced after preview");
        let error = apply_client_command(command, &fake_sidecar(), true).unwrap_err();
        assert!(error.contains("executable changed after preview"));
        assert_eq!(fs::read(&target).unwrap(), b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn client_config_changed_after_preview_is_rejected() {
        let dir = tempdir().unwrap();
        let script = write_executable_script(dir.path(), "fake-cli", "exit 0");
        let target = dir.path().join(".claude.json");
        let before = b"{}".to_vec();
        fs::write(&target, &before).unwrap();
        let command = test_command(&script, vec![], target.clone(), Some(before));
        fs::write(&target, b"{\"changed\":true}").unwrap();
        let error = apply_client_command(command, &fake_sidecar(), true).unwrap_err();
        assert!(error.contains("config changed after preview"));
    }

    #[cfg(unix)]
    #[test]
    fn client_cli_exits_nonzero_rolls_back_config() {
        let dir = tempdir().unwrap();
        let script = write_executable_script(
            dir.path(),
            "fake-cli",
            "printf 'raw stderr line' >&2\nexit 3",
        );
        let target = dir.path().join(".claude.json");
        let before = b"{}".to_vec();
        fs::write(&target, &before).unwrap();
        let command = test_command(&script, vec![], target.clone(), Some(before));
        let error = apply_client_command(command, &fake_sidecar(), true).unwrap_err();
        assert!(error.contains("rolled back"));
        assert!(
            !error.contains("raw stderr line"),
            "raw output must not leak"
        );
        assert_eq!(fs::read(&target).unwrap(), b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn client_cli_that_hangs_is_killed_within_timeout() {
        let dir = tempdir().unwrap();
        // Use a shell-builtin infinite loop so the timeout test does not
        // depend on an external `sleep` executable or its platform behavior.
        let script = write_executable_script(dir.path(), "fake-cli", "while :; do :; done");
        let started = Instant::now();
        let (status, _, _, timed_out) = run_with_timeout(
            &script,
            &[],
            Duration::from_millis(400),
            MAX_CAPTURED_OUTPUT,
        );
        assert!(timed_out);
        assert!(status.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "hung child must be killed promptly, took {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn client_cli_output_is_capped_at_eight_kibibytes() {
        let dir = tempdir().unwrap();
        let script = write_executable_script(
            dir.path(),
            "fake-cli",
            "head -c 200000 /dev/zero | tr '\\0' 'x'; head -c 200000 /dev/zero | tr '\\0' 'y' >&2",
        );
        let (status, stdout, stderr, timed_out) =
            run_with_timeout(&script, &[], Duration::from_secs(10), MAX_CAPTURED_OUTPUT);
        assert!(!timed_out);
        assert!(status.is_some() && status.unwrap().success());
        assert!(stdout.len() <= MAX_CAPTURED_OUTPUT);
        assert!(stderr.len() <= MAX_CAPTURED_OUTPUT);
    }

    #[cfg(unix)]
    #[test]
    fn client_command_rollback_succeeds() {
        let dir = tempdir().unwrap();
        let target = dir.path().join(".claude.json");
        let before = b"{}".to_vec();
        fs::write(&target, &before).unwrap();
        let script = write_executable_script(
            dir.path(),
            "fake-cli",
            "printf '%s\\n' '/verified/Infimount Sidecar/mcp' > \"$1\"",
        );
        let command = test_command(
            &script,
            vec![target.to_string_lossy().to_string()],
            target.clone(),
            Some(before.clone()),
        );
        let result = apply_client_command(command, &fake_sidecar(), true).unwrap();
        assert!(result.applied);
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap())
            .contains("/verified/Infimount Sidecar/mcp"));
        rollback_mcp_client_install(result.rollback_id.unwrap()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), before);
    }
}
