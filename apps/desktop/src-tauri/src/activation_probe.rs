use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use chrono::Utc;
use infimount_core::workspaces::{WorkspaceRecord, WorkspaceRegistry};
use infimount_mcp::policy::{McpAccessMode, McpRuleSource};
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use infimount_mcp::telemetry::{build_os_arch, ProductEvent, ProductEventName, ProductEventStore};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_COMMAND_OUTPUT: usize = 8 * 1024;
const ADMIN_TOOLS: &[&str] = &[
    "list_storages",
    "add_storage",
    "edit_storage",
    "remove_storage",
    "import_config",
    "export_config",
    "validate_storage",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarValidation {
    pub binary_found: bool,
    pub executable: bool,
    pub version: Option<String>,
    pub version_match: bool,
    pub doctor_healthy: bool,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationProbeOutput {
    pub sidecar: SidecarValidation,
    pub mcp_handshake_ok: bool,
    pub mcp_allowed_op_ok: bool,
    pub mcp_denial_proven: bool,
    pub overall_ok: bool,
    pub error_code: Option<String>,
}

#[derive(Debug)]
struct LocatedSidecar {
    path: PathBuf,
    version: String,
}

#[derive(Debug)]
struct ProbeTarget {
    workspace_path: String,
    inside_path: String,
    outside_path: String,
    inside_expected: String,
    config_home: Option<PathBuf>,
    config_appdata: Option<PathBuf>,
}

struct ChildGuard {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

impl ChildGuard {
    fn new(child: Child, stdin: ChildStdin) -> Self {
        Self {
            child: Some(child),
            stdin: Some(stdin),
        }
    }

    fn write_json(&mut self, value: &Value) -> Result<(), &'static str> {
        let stdin = self.stdin.as_mut().ok_or("ERR_MCP_HANDSHAKE_FAILED")?;
        serde_json::to_writer(&mut *stdin, value).map_err(|_| "ERR_MCP_HANDSHAKE_FAILED")?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|_| "ERR_MCP_HANDSHAKE_FAILED")
    }

    fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_none()
    }

    fn stop(&mut self) {
        self.stdin.take();
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + GRACEFUL_STOP_TIMEOUT;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn target_triple() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else {
        "unsupported-target"
    }
}

fn executable_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else {
        ""
    }
}

fn installed_sidecar_filename() -> String {
    format!("mcp{}", executable_suffix())
}

fn development_sidecar_filename() -> String {
    format!("mcp-{}{}", target_triple(), executable_suffix())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
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

fn sidecar_candidates() -> Vec<PathBuf> {
    // The override exists only in test binaries so packaged production activation can
    // never be redirected to an arbitrary executable outside the app bundle.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("INFIMOUNT_MCP_PATH").map(PathBuf::from) {
        if path.is_absolute() {
            return vec![path];
        }
    }
    let mut candidates = Vec::new();
    let installed = installed_sidecar_filename();
    let development = development_sidecar_filename();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for root in [
                dir.to_path_buf(),
                dir.join("binaries"),
                dir.join("resources"),
                dir.join("resources").join("binaries"),
            ] {
                candidates.push(root.join(&installed));
                candidates.push(root.join(&development));
            }
        }
    }
    let manifest_binaries = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    candidates.push(manifest_binaries.join(&development));
    candidates.push(manifest_binaries.join(&installed));
    candidates.dedup();
    candidates
}

fn bounded_command(path: &Path, args: &[&str]) -> Result<(bool, String), &'static str> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "ERR_SIDECAR_NOT_EXECUTABLE")?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("ERR_SIDECAR_TIMEOUT");
            }
            Err(_) => return Err("ERR_SIDECAR_EXECUTION_FAILED"),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|_| "ERR_SIDECAR_EXECUTION_FAILED")?;
    if output.stdout.len() > MAX_COMMAND_OUTPUT || output.stderr.len() > MAX_COMMAND_OUTPUT {
        return Err("ERR_SIDECAR_OUTPUT_TOO_LARGE");
    }
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn locate_same_version_sidecar() -> Result<LocatedSidecar, &'static str> {
    locate_same_version_sidecar_from(sidecar_candidates())
}

fn locate_same_version_sidecar_from(
    candidates: Vec<PathBuf>,
) -> Result<LocatedSidecar, &'static str> {
    let expected_version = env!("CARGO_PKG_VERSION");
    let mut saw_file = false;
    let mut saw_executable = false;
    let mut last_execution_error = None;
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        saw_file = true;
        if !is_executable_file(&path) {
            continue;
        }
        saw_executable = true;
        match bounded_command(&path, &["--version"]) {
            Ok((true, output)) => {
                let version = output
                    .strip_prefix("infimount_mcp ")
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if version == Some(expected_version) {
                    let path = std::fs::canonicalize(path).map_err(|_| "ERR_SIDECAR_NOT_FOUND")?;
                    return Ok(LocatedSidecar {
                        path,
                        version: expected_version.to_string(),
                    });
                }
            }
            Ok((false, _)) => last_execution_error = Some("ERR_SIDECAR_EXECUTION_FAILED"),
            Err(code) => last_execution_error = Some(code),
        }
    }
    if !saw_file {
        Err("ERR_SIDECAR_NOT_FOUND")
    } else if !saw_executable {
        Err("ERR_SIDECAR_NOT_EXECUTABLE")
    } else if let Some(code) = last_execution_error {
        Err(code)
    } else {
        Err("ERR_SIDECAR_VERSION_MISMATCH")
    }
}

fn validate_and_locate_sidecar() -> Result<LocatedSidecar, SidecarValidation> {
    let located = locate_same_version_sidecar().map_err(|code| SidecarValidation {
        binary_found: code != "ERR_SIDECAR_NOT_FOUND",
        executable: !matches!(code, "ERR_SIDECAR_NOT_FOUND" | "ERR_SIDECAR_NOT_EXECUTABLE"),
        version: None,
        version_match: false,
        doctor_healthy: false,
        error_code: Some(code.to_string()),
    })?;
    let doctor_healthy = bounded_command(&located.path, &["doctor", "--json"])
        .ok()
        .filter(|(success, _)| *success)
        .and_then(|(_, output)| serde_json::from_str::<Value>(&output).ok())
        .and_then(|report| report.get("healthy").and_then(Value::as_bool))
        == Some(true);
    if !doctor_healthy {
        return Err(SidecarValidation {
            binary_found: true,
            executable: true,
            version: Some(located.version.clone()),
            version_match: true,
            doctor_healthy: false,
            error_code: Some("ERR_SIDECAR_DOCTOR_FAILED".to_string()),
        });
    }
    Ok(located)
}

pub(crate) fn verified_sidecar_path() -> Result<PathBuf, &'static str> {
    validate_and_locate_sidecar()
        .map(|located| located.path)
        .map_err(|validation| match validation.error_code.as_deref() {
            Some("ERR_SIDECAR_NOT_FOUND") => "ERR_SIDECAR_NOT_FOUND",
            Some("ERR_SIDECAR_NOT_EXECUTABLE") => "ERR_SIDECAR_NOT_EXECUTABLE",
            Some("ERR_SIDECAR_VERSION_MISMATCH") => "ERR_SIDECAR_VERSION_MISMATCH",
            Some("ERR_SIDECAR_DOCTOR_FAILED") => "ERR_SIDECAR_DOCTOR_FAILED",
            Some("ERR_SIDECAR_TIMEOUT") => "ERR_SIDECAR_TIMEOUT",
            _ => "ERR_SIDECAR_EXECUTION_FAILED",
        })
}

pub fn validate_sidecar_binary() -> SidecarValidation {
    match validate_and_locate_sidecar() {
        Ok(located) => SidecarValidation {
            binary_found: true,
            executable: true,
            version: Some(located.version),
            version_match: true,
            doctor_healthy: true,
            error_code: None,
        },
        Err(validation) => validation,
    }
}

pub fn record_startup_sidecar_event(
    product_events: &ProductEventStore,
    validation: &SidecarValidation,
) {
    let success = validation.version_match && validation.doctor_healthy;
    let event = ProductEvent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now().to_rfc3339(),
        name: ProductEventName::SidecarVerified,
        schema_version: 1,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_arch: build_os_arch(),
        backend_type: None,
        workspace_template: None,
        access_profile: None,
        client_kind: None,
        success: Some(success),
        failure_stage: (!success).then(|| "startup".to_string()),
        error_code: (!success)
            .then(|| validation.error_code.clone())
            .flatten()
            .or_else(|| (!success).then(|| "ERR_SIDECAR_UNHEALTHY".to_string())),
        duration_bucket: None,
    };
    let _ = product_events.record(event);
}

fn normalize_relative_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| *part == ".." || part.contains('%')) {
        return None;
    }
    Some(parts.join("/"))
}

fn workspace_rule_corresponds(storage: &StorageRecord, workspace: &WorkspaceRecord) -> bool {
    let Some(root_path) = normalize_relative_path(&workspace.root_path) else {
        return false;
    };
    storage.mcp_policy.rules.iter().any(|rule| {
        let source_matches = matches!(
            &rule.source,
            McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
        );
        let id_matches = workspace
            .policy_rule_id
            .as_ref()
            .is_none_or(|expected| expected == &rule.id);
        source_matches
            && id_matches
            && matches!(
                rule.access,
                McpAccessMode::ReadOnly | McpAccessMode::ReadWrite
            )
            && normalize_relative_path(&rule.prefix).as_deref() == Some(root_path.as_str())
    })
}

fn select_probe_target(registry: &StorageRegistry) -> Result<ProbeTarget, &'static str> {
    let config_dir = registry
        .path()
        .parent()
        .ok_or("ERR_ACTIVATION_STORAGE_UNAVAILABLE")?;
    let workspaces = WorkspaceRegistry::new(config_dir)
        .load_all()
        .map_err(|_| "ERR_ACTIVATION_WORKSPACE_UNAVAILABLE")?;
    let storages = registry
        .load_all()
        .map_err(|_| "ERR_ACTIVATION_STORAGE_UNAVAILABLE")?;

    for workspace in &workspaces {
        let Some(storage) = storages.iter().find(|storage| {
            storage.id == workspace.storage_id
                && storage.enabled
                && storage.mcp_exposed
                && storage.backend == "local"
        }) else {
            continue;
        };
        if !workspace_rule_corresponds(storage, workspace) {
            continue;
        }
        let Some(storage_root) = storage.config.get("root").and_then(Value::as_str) else {
            continue;
        };
        let Some(workspace_root) = normalize_relative_path(&workspace.root_path) else {
            continue;
        };
        let inside_relative = format!("{workspace_root}/sample.txt");
        let outside_relative = "outside/denied.txt".to_string();
        let storage_root = PathBuf::from(storage_root);
        let inside_fixture = storage_root.join(&inside_relative);
        let outside_fixture = storage_root.join(&outside_relative);
        if !inside_fixture.is_file() || !outside_fixture.is_file() {
            continue;
        }
        let Ok(inside_expected) = std::fs::read_to_string(&inside_fixture) else {
            continue;
        };
        if inside_expected.len() > 2_097_152 {
            continue;
        }
        let virtual_root = format!("/{}/{}", storage.name, workspace_root);
        let config_parent = config_dir.parent().map(Path::to_path_buf);
        return Ok(ProbeTarget {
            workspace_path: virtual_root,
            inside_path: format!("/{}/{}", storage.name, inside_relative),
            outside_path: format!("/{}/{}", storage.name, outside_relative),
            inside_expected,
            config_home: if cfg!(windows) {
                None
            } else {
                config_parent.clone()
            },
            config_appdata: if cfg!(windows) { config_parent } else { None },
        });
    }
    Err("ERR_ACTIVATION_WORKSPACE_FIXTURES_REQUIRED")
}

pub async fn run_activation_probe(
    registry: StorageRegistry,
    product_events: &ProductEventStore,
) -> ActivationProbeOutput {
    let started = Instant::now();
    let sidecar = validate_sidecar_binary();
    let probe_result = if sidecar.version_match && sidecar.doctor_healthy {
        match (
            locate_same_version_sidecar(),
            select_probe_target(&registry),
        ) {
            (Ok(located), Ok(target)) => tokio::time::timeout(TOTAL_PROBE_TIMEOUT, async move {
                tokio::task::spawn_blocking(move || run_sidecar_probe(&located.path, &target))
                    .await
                    .map_err(|_| "ERR_ACTIVATION_PROBE_FAILED")?
            })
            .await
            .map_err(|_| "ERR_ACTIVATION_TIMEOUT")
            .and_then(|result| result),
            (Err(code), _) | (_, Err(code)) => Err(code),
        }
    } else {
        Err(sidecar
            .error_code
            .as_deref()
            .unwrap_or("ERR_SIDECAR_UNHEALTHY"))
    };

    let (handshake, allowed, denied, error_code) = match probe_result {
        Ok(()) => (true, true, true, None),
        Err(code) => {
            let handshake = matches!(
                code,
                "ERR_ACTIVATION_TOOL_LIST_FAILED"
                    | "ERR_ACTIVATION_ADMIN_TOOL_EXPOSED"
                    | "ERR_ACTIVATION_SAFE_TOOLS_MISSING"
                    | "ERR_ACTIVATION_ALLOWED_OP_FAILED"
                    | "ERR_ACTIVATION_DENIAL_CHECK_FAILED"
                    | "ERR_ACTIVATION_POLICY_NOT_ENFORCED"
            );
            let allowed = matches!(
                code,
                "ERR_ACTIVATION_DENIAL_CHECK_FAILED" | "ERR_ACTIVATION_POLICY_NOT_ENFORCED"
            );
            (handshake, allowed, false, Some(code.to_string()))
        }
    };
    let overall_ok = sidecar.version_match && sidecar.doctor_healthy && probe_result.is_ok();
    record_probe_events(
        product_events,
        &sidecar,
        overall_ok,
        error_code.as_deref(),
        started.elapsed(),
    );

    ActivationProbeOutput {
        sidecar,
        mcp_handshake_ok: handshake,
        mcp_allowed_op_ok: allowed,
        mcp_denial_proven: denied,
        overall_ok,
        error_code,
    }
}

fn run_sidecar_probe(path: &Path, target: &ProbeTarget) -> Result<(), &'static str> {
    let mut command = Command::new(path);
    command
        .args(["serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = &target.config_home {
        command.env("HOME", home);
    }
    if let Some(appdata) = &target.config_appdata {
        command.env("APPDATA", appdata);
    }
    let mut child = command.spawn().map_err(|_| "ERR_SIDECAR_START_FAILED")?;
    let stdin = child.stdin.take().ok_or("ERR_SIDECAR_START_FAILED")?;
    let stdout = child.stdout.take().ok_or("ERR_SIDECAR_START_FAILED")?;
    let stderr = child.stderr.take().ok_or("ERR_SIDECAR_START_FAILED")?;
    let (sender, receiver) = mpsc::sync_channel::<Result<String, ()>>(32);
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|_| ());
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let mut bounded = Vec::new();
        let _ = stderr
            .take((MAX_COMMAND_OUTPUT + 1) as u64)
            .read_to_end(&mut bounded);
    });
    let mut guard = ChildGuard::new(child, stdin);
    let total_deadline = Instant::now() + TOTAL_PROBE_TIMEOUT;

    guard.write_json(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "infimount-activation", "version": env!("CARGO_PKG_VERSION")}
        }
    }))?;
    let initialize = receive_response(
        &receiver,
        &mut guard,
        1,
        STARTUP_TIMEOUT,
        total_deadline,
        "ERR_MCP_HANDSHAKE_FAILED",
    )?;
    if initialize.get("result").is_none() || initialize.get("error").is_some() {
        return Err("ERR_MCP_HANDSHAKE_FAILED");
    }
    guard.write_json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;

    guard.write_json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))?;
    let tools = receive_response(
        &receiver,
        &mut guard,
        2,
        REQUEST_TIMEOUT,
        total_deadline,
        "ERR_ACTIVATION_TOOL_LIST_FAILED",
    )?;
    let tool_names = tools
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or("ERR_ACTIVATION_TOOL_LIST_FAILED")?
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if ADMIN_TOOLS.iter().any(|admin| tool_names.contains(admin)) {
        return Err("ERR_ACTIVATION_ADMIN_TOOL_EXPOSED");
    }
    if !["list_dir", "read_file"]
        .iter()
        .all(|required| tool_names.contains(required))
    {
        return Err("ERR_ACTIVATION_SAFE_TOOLS_MISSING");
    }

    let list = call_tool(
        &receiver,
        &mut guard,
        3,
        "list_dir",
        json!({"path": target.workspace_path, "limit": 100}),
        total_deadline,
    )?;
    if tool_call_failed(&list) || !value_contains_text(&list, "sample.txt") {
        return Err("ERR_ACTIVATION_ALLOWED_OP_FAILED");
    }

    let read_inside = call_tool(
        &receiver,
        &mut guard,
        4,
        "read_file",
        json!({"path": target.inside_path, "as_text": true}),
        total_deadline,
    )?;
    if tool_call_failed(&read_inside) || !value_contains_text(&read_inside, &target.inside_expected)
    {
        return Err("ERR_ACTIVATION_ALLOWED_OP_FAILED");
    }

    let read_outside = call_tool(
        &receiver,
        &mut guard,
        5,
        "read_file",
        json!({"path": target.outside_path, "as_text": true}),
        total_deadline,
    )
    .map_err(|_| "ERR_ACTIVATION_DENIAL_CHECK_FAILED")?;
    if find_structured_error_code(&read_outside) != Some("ERR_MCP_POLICY_DENIED") {
        return Err("ERR_ACTIVATION_POLICY_NOT_ENFORCED");
    }

    guard.stop();
    Ok(())
}

fn call_tool(
    receiver: &Receiver<Result<String, ()>>,
    guard: &mut ChildGuard,
    id: i64,
    name: &str,
    arguments: Value,
    total_deadline: Instant,
) -> Result<Value, &'static str> {
    guard
        .write_json(&json!({
            "jsonrpc":"2.0", "id":id, "method":"tools/call",
            "params":{"name":name,"arguments":arguments}
        }))
        .map_err(|_| "ERR_ACTIVATION_ALLOWED_OP_FAILED")?;
    receive_response(
        receiver,
        guard,
        id,
        REQUEST_TIMEOUT,
        total_deadline,
        "ERR_ACTIVATION_ALLOWED_OP_FAILED",
    )
}

fn receive_response(
    receiver: &Receiver<Result<String, ()>>,
    guard: &mut ChildGuard,
    id: i64,
    timeout: Duration,
    total_deadline: Instant,
    error: &'static str,
) -> Result<Value, &'static str> {
    let deadline = std::cmp::min(Instant::now() + timeout, total_deadline);
    loop {
        if !guard.is_running() {
            return Err(error);
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(error)?;
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|_| error)?
            .map_err(|_| error)?;
        if line.len() > MAX_COMMAND_OUTPUT {
            return Err(error);
        }
        let value = serde_json::from_str::<Value>(&line).map_err(|_| error)?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

fn tool_call_failed(value: &Value) -> bool {
    value.get("error").is_some()
        || value
            .pointer("/result/isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || find_structured_error_code(value).is_some()
}

fn find_structured_error_code(value: &Value) -> Option<&str> {
    match value {
        Value::Object(map) => {
            if let Some(code) = map.get("code").and_then(Value::as_str) {
                if code.starts_with("ERR_") {
                    return Some(code);
                }
            }
            map.values().find_map(find_structured_error_code)
        }
        Value::Array(values) => values.iter().find_map(find_structured_error_code),
        _ => None,
    }
}

fn value_contains_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text.contains(expected),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_text(value, expected)),
        Value::Object(map) => map
            .values()
            .any(|value| value_contains_text(value, expected)),
        _ => false,
    }
}

fn record_probe_events(
    store: &ProductEventStore,
    sidecar: &SidecarValidation,
    success: bool,
    error_code: Option<&str>,
    duration: Duration,
) {
    let duration_bucket = if duration < Duration::from_secs(1) {
        "fast"
    } else if duration < Duration::from_secs(5) {
        "moderate"
    } else {
        "slow"
    };
    let common_id = Uuid::new_v4().to_string();
    let timestamp = Utc::now().to_rfc3339();
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = build_os_arch();
    let _ = store.record(ProductEvent {
        id: format!("{common_id}-sidecar"),
        timestamp: timestamp.clone(),
        name: ProductEventName::SidecarVerified,
        schema_version: 1,
        app_version: app_version.clone(),
        os_arch: os_arch.clone(),
        backend_type: None,
        workspace_template: None,
        access_profile: None,
        client_kind: None,
        success: Some(sidecar.version_match && sidecar.doctor_healthy),
        failure_stage: sidecar
            .error_code
            .as_ref()
            .map(|_| "sidecar_validation".to_string()),
        error_code: sidecar.error_code.clone(),
        duration_bucket: Some(duration_bucket.to_string()),
    });
    let _ = store.record(ProductEvent {
        id: format!("{common_id}-probe"),
        timestamp,
        name: ProductEventName::McpProbeCompleted,
        schema_version: 1,
        app_version,
        os_arch,
        backend_type: None,
        workspace_template: None,
        access_profile: None,
        client_kind: None,
        success: Some(success),
        failure_stage: (!success).then(|| "activation_probe".to_string()),
        error_code: error_code.map(ToOwned::to_owned),
        duration_bucket: Some(duration_bucket.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use infimount_core::secrets::MemorySecretStore;
    use infimount_core::workspaces::{WorkspaceRecord, WorkspaceRegistry};
    use infimount_mcp::policy::{McpAccessMode, McpPathRule, McpRuleSource, McpStoragePolicy};
    use infimount_mcp::registry::{StorageRecord, StorageRegistry};

    use super::*;

    #[test]
    fn locator_candidates_include_installed_and_development_names() {
        let candidates = sidecar_candidates();
        assert!(candidates.iter().any(|path| {
            path.file_name().and_then(|name| name.to_str())
                == Some(installed_sidecar_filename().as_str())
        }));
        assert!(candidates.iter().any(|path| {
            path.file_name().and_then(|name| name.to_str())
                == Some(development_sidecar_filename().as_str())
        }));
    }

    #[test]
    fn rejects_nonexistent_or_unverified_sidecar_without_panicking() {
        let result = validate_sidecar_binary();
        if result.version_match {
            assert!(result.version.is_some());
        }
        if result.doctor_healthy {
            assert!(result.version_match);
        }
        assert_eq!(
            locate_same_version_sidecar_from(Vec::new()).unwrap_err(),
            "ERR_SIDECAR_NOT_FOUND"
        );
    }

    #[cfg(unix)]
    #[test]
    fn locator_rejects_non_executable_and_wrong_version_binaries() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("mcp");
        std::fs::write(&path, "#!/bin/sh\necho 'infimount_mcp 0.0.0'\n")
            .expect("write sidecar fixture");
        assert_eq!(
            locate_same_version_sidecar_from(vec![path.clone()]).unwrap_err(),
            "ERR_SIDECAR_NOT_EXECUTABLE"
        );
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(
            locate_same_version_sidecar_from(vec![path]).unwrap_err(),
            "ERR_SIDECAR_VERSION_MISMATCH"
        );
    }

    #[test]
    fn extracts_only_structured_policy_error() {
        let value = json!({"result":{"structuredContent":{"ok":false,"error":{"code":"ERR_MCP_POLICY_DENIED"}}}});
        assert_eq!(
            find_structured_error_code(&value),
            Some("ERR_MCP_POLICY_DENIED")
        );
        assert_eq!(
            find_structured_error_code(&json!({"message":"ERR_MCP_POLICY_DENIED"})),
            None
        );
    }

    #[test]
    fn workspace_target_requires_registry_correspondence_and_real_fixtures() {
        let fixture = create_demo_fixture();
        let target = select_probe_target(&fixture.registry).expect("valid target");
        assert!(target.workspace_path.ends_with("/workspace"));
        assert!(target.inside_path.ends_with("/workspace/sample.txt"));
        assert!(target.outside_path.ends_with("/outside/denied.txt"));
        assert_eq!(target.inside_expected, "inside fixture\n");

        fixture
            .workspace_registry
            .delete("workspace-id")
            .expect("remove correspondence");
        assert_eq!(
            select_probe_target(&fixture.registry).unwrap_err(),
            "ERR_ACTIVATION_WORKSPACE_FIXTURES_REQUIRED"
        );
    }

    #[test]
    #[ignore = "runs the prepared packaged sidecar; use scripts/smoke-activation.sh"]
    fn complete_demo_activation_over_packaged_stdio_sidecar() {
        let binary = std::env::var_os("INFIMOUNT_MCP_PATH")
            .map(PathBuf::from)
            .expect("INFIMOUNT_MCP_PATH must name the prepared sidecar");
        assert!(is_executable_file(&binary));
        let fixture = create_demo_fixture();
        let target = select_probe_target(&fixture.registry).expect("valid activation target");
        run_sidecar_probe(&binary, &target).expect("complete stdio activation proof");
    }

    struct DemoFixture {
        _temp: tempfile::TempDir,
        registry: StorageRegistry,
        workspace_registry: WorkspaceRegistry,
    }

    fn create_demo_fixture() -> DemoFixture {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_dir = temp.path().join(".infimount");
        let storage_root = temp.path().join("demo");
        std::fs::create_dir_all(storage_root.join("workspace")).expect("workspace dir");
        std::fs::create_dir_all(storage_root.join("outside")).expect("outside dir");
        std::fs::write(storage_root.join("workspace/README.md"), "# Demo\n")
            .expect("README fixture");
        std::fs::write(
            storage_root.join("workspace/sample.txt"),
            "inside fixture\n",
        )
        .expect("inside fixture");
        std::fs::write(storage_root.join("outside/denied.txt"), "outside fixture\n")
            .expect("outside fixture");

        let secret_store = Arc::new(MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(config_dir.join("storages.json")),
            secret_store,
        );
        let mut storage = StorageRecord::new(
            "Demo".to_string(),
            "local".to_string(),
            json!({"root": storage_root}),
        );
        storage.id = "storage-id".to_string();
        storage.mcp_exposed = true;
        storage.read_only = true;
        storage.mcp_policy = McpStoragePolicy {
            default_access: McpAccessMode::None,
            rules: vec![McpPathRule {
                id: "rule-id".to_string(),
                prefix: "workspace".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "workspace-id".to_string(),
                },
                confirmation_rules: None,
            }],
            denied_paths: vec!["outside".to_string()],
            ..McpStoragePolicy::default()
        };
        registry
            .save_all_atomic(&[storage])
            .expect("save storage registry");

        let workspace_registry = WorkspaceRegistry::new(&config_dir);
        workspace_registry
            .create(&WorkspaceRecord {
                id: "workspace-id".to_string(),
                schema_version: 1,
                storage_id: "storage-id".to_string(),
                name: "Demo workspace".to_string(),
                root_path: "workspace".to_string(),
                template_id: "custom".to_string(),
                access_profile: "read_only".to_string(),
                policy_rule_id: Some("rule-id".to_string()),
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
                memory_files: vec![],
                checkpoint_ids: vec![],
            })
            .expect("save workspace registry");
        std::fs::write(
            config_dir.join("mcp_settings.json"),
            serde_json::to_vec_pretty(&infimount_mcp::settings::McpSettings::default()).unwrap(),
        )
        .expect("save MCP settings");

        DemoFixture {
            _temp: temp,
            registry,
            workspace_registry,
        }
    }
}
