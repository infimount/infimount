use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use infimount_core::agent_task_io::hash_agent_task_file;
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASK_BRIEF_FILE,
    AGENT_TASK_INPUTS_DIR, AGENT_TASK_MANIFEST_FILE, AGENT_TASK_OUTPUTS_DIR,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{CoreError, SourceKind};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const CODEX_CLIENT_NAME: &str = "codex";
const REQUIRED_CODEX_MCP_TOOLS: &[&str] = &["list_dir", "stat_path", "read_file", "write_file"];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskCodexHandoffRequest {
    pub workspace_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskCodexHandoffOutput {
    pub client: &'static str,
    pub workspace_id: String,
    pub workspace_name: String,
    pub task_id: String,
    pub task_root: String,
    pub launched: bool,
}

#[tauri::command]
pub async fn launch_agent_task_in_codex(
    state: State<'_, AppState>,
    request: AgentTaskCodexHandoffRequest,
) -> Result<AgentTaskCodexHandoffOutput, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config("Agent Task handoff is unavailable while Infimount is degraded".into())
    })?;

    Uuid::parse_str(&request.task_id)
        .map_err(|_| CoreError::Config("Agent Task handoff requires a valid task id".into()))?;
    Uuid::parse_str(&request.workspace_id).map_err(|_| {
        CoreError::Config("Agent Task handoff requires a valid workspace id".into())
    })?;

    let workspace = state
        .workspaces
        .find_by_id(&request.workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }
    if workspace.access_profile != "read_write" {
        return Err(CoreError::Config(
            "Codex handoff requires a read-write Agent Workspace".into(),
        ));
    }
    let policy_rule_id = workspace.policy_rule_id.clone().ok_or_else(|| {
        CoreError::Config(
            "Codex handoff requires the Agent Workspace MCP policy to be applied".into(),
        )
    })?;

    let storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(|_| {
            CoreError::Config("Agent Task workspace storage could not be validated".into())
        })?;
    if !storage.enabled || storage.read_only {
        return Err(CoreError::Config(
            "Codex handoff requires an enabled writable workspace storage".into(),
        ));
    }
    if !storage.mcp_exposed {
        return Err(CoreError::Config(
            "Codex handoff requires the Agent Workspace storage to be exposed to MCP".into(),
        ));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Codex handoff currently requires a Local Filesystem Agent Workspace".into(),
        ));
    }

    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".into()))?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let task_root = agent_task_root(&request.task_id)?;
    let workspace_task_path = join_path(&workspace.root_path, &task_root);
    let outputs_path = join_path(&workspace_task_path, AGENT_TASK_OUTPUTS_DIR);
    let scope = infimount_mcp::AgentTaskScope::new(
        request.task_id.clone(),
        storage.id.clone(),
        storage.name.clone(),
        workspace.id.clone(),
        policy_rule_id,
        namespace,
        workspace.root_path.clone(),
        workspace_task_path.clone(),
        outputs_path.clone(),
    )
    .map_err(|_| {
        CoreError::Config("Agent Workspace MCP policy does not match this prepared task".into())
    })?;
    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| {
            CoreError::Config(
                "Agent Workspace MCP policy no longer grants this workspace read-write access"
                    .into(),
            )
        })?;

    let settings = state
        .settings_store
        .load()
        .map_err(|_| CoreError::Config("Infimount MCP settings could not be validated".into()))?;
    for required in REQUIRED_CODEX_MCP_TOOLS {
        if !settings.enabled_tools.iter().any(|tool| tool == required) {
            return Err(CoreError::Config(format!(
                "Codex handoff requires the {required} MCP tool to be enabled in MCP Settings"
            )));
        }
    }

    validate_local_path(&storage, &workspace.root_path)?;
    validate_local_path(&storage, &workspace_task_path)?;

    let op = state.operator_for_storage_id(&storage.id)?;
    require_directory(&op, &workspace_task_path, "Agent Task package").await?;

    let manifest_path = join_path(&workspace_task_path, AGENT_TASK_MANIFEST_FILE);
    let brief_path = join_path(&workspace_task_path, AGENT_TASK_BRIEF_FILE);
    for path in [&manifest_path, &brief_path, &outputs_path] {
        validate_local_path(&storage, path)?;
    }
    require_file(&op, &brief_path, "Agent Task brief").await?;
    require_directory(&op, &outputs_path, "Agent Task outputs directory").await?;

    let manifest_metadata = op.stat(&manifest_path).await?;
    if !manifest_metadata.is_file() {
        return Err(CoreError::Config(
            "Agent Task manifest is missing or is not a file".into(),
        ));
    }
    if manifest_metadata.content_length() > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest_bytes = op.read(&manifest_path).await?.to_vec();
    if manifest_bytes.len() as u64 > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest: AgentTaskManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_agent_task_manifest(&manifest)?;
    if manifest.task_id != request.task_id
        || manifest.workspace_id != request.workspace_id
        || manifest.task_root != task_root
    {
        return Err(CoreError::Config(
            "Agent Task manifest does not match the requested workspace task".into(),
        ));
    }

    verify_prepared_inputs(&op, &storage, &workspace_task_path, &manifest).await?;

    // Re-run confinement and policy binding immediately before handing the task to an
    // external client. Codex never receives the workspace's host filesystem path.
    validate_local_path(&storage, &workspace_task_path)?;
    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| {
            CoreError::Config(
                "Agent Workspace MCP policy changed before Codex handoff; review it and retry"
                    .into(),
            )
        })?;

    let sidecar = crate::activation_probe::verified_sidecar_path().map_err(|_| {
        CoreError::Config("The verified Infimount MCP sidecar is unavailable".into())
    })?;
    let mcp_task_path = format!("/{}/{}", storage.name, scope.task_prefix.trim_matches('/'));
    let mcp_outputs_path = format!(
        "/{}/{}",
        storage.name,
        scope.outputs_prefix.trim_matches('/')
    );
    let prompt = codex_task_prompt(&workspace.name, &mcp_task_path, &mcp_outputs_path);
    let neutral_cwd = create_neutral_handoff_directory()?;
    let args = match codex_arguments(&sidecar, &neutral_cwd, &prompt, &scope) {
        Ok(args) => args,
        Err(error) => {
            cleanup_neutral_directory(&neutral_cwd);
            return Err(error);
        }
    };
    launch_codex_terminal(&args, &neutral_cwd)?;

    Ok(AgentTaskCodexHandoffOutput {
        client: CODEX_CLIENT_NAME,
        workspace_id: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        task_id: request.task_id,
        task_root,
        launched: true,
    })
}

async fn verify_prepared_inputs(
    op: &opendal::Operator,
    storage: &infimount_mcp::registry::StorageRecord,
    workspace_task_path: &str,
    manifest: &AgentTaskManifest,
) -> Result<(), CoreError> {
    let inputs_root = join_path(workspace_task_path, AGENT_TASK_INPUTS_DIR);
    validate_local_path(storage, &inputs_root)?;
    require_directory(op, &inputs_root, "Agent Task inputs directory").await?;

    let entries = infimount_core::operations::list_entries_recursive(op, &inputs_root)
        .await
        .map_err(|_| {
            CoreError::Config("Agent Task prepared input set could not be verified".into())
        })?;
    if entries.len() >= infimount_core::models::MAX_RECURSIVE_ITEMS as usize {
        return Err(CoreError::Config(
            "Agent Task prepared input set is too large to verify without truncation".into(),
        ));
    }
    let actual_files = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.path.trim_matches('/').to_string())
        .collect::<HashSet<_>>();
    let expected_files = expected_prepared_input_paths(workspace_task_path, manifest);

    if actual_files != expected_files {
        return Err(CoreError::Config(
            "Agent Task prepared input set changed after preparation".into(),
        ));
    }

    let mut inputs = manifest.inputs.iter().collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));
    for input in inputs {
        let path = join_path(workspace_task_path, &input.task_path);
        validate_local_path(storage, &path)?;
        let digest = hash_agent_task_file(op, &path).await.map_err(|_| {
            CoreError::Config("Agent Task prepared input bytes could not be verified".into())
        })?;
        if digest.byte_size != input.byte_size || digest.sha256 != input.sha256 {
            return Err(CoreError::Config(
                "Agent Task prepared input bytes changed after preparation".into(),
            ));
        }
    }

    Ok(())
}

fn expected_prepared_input_paths(
    workspace_task_path: &str,
    manifest: &AgentTaskManifest,
) -> HashSet<String> {
    manifest
        .inputs
        .iter()
        .map(|input| {
            join_path(workspace_task_path, &input.task_path)
                .trim_matches('/')
                .to_string()
        })
        .collect()
}

async fn require_directory(
    op: &opendal::Operator,
    path: &str,
    label: &str,
) -> Result<(), CoreError> {
    let metadata = op
        .stat(path)
        .await
        .map_err(|_| CoreError::Config(format!("{label} could not be read")))?;
    if !metadata.is_dir() {
        return Err(CoreError::Config(format!("{label} is not a directory")));
    }
    Ok(())
}

async fn require_file(op: &opendal::Operator, path: &str, label: &str) -> Result<(), CoreError> {
    let metadata = op
        .stat(path)
        .await
        .map_err(|_| CoreError::Config(format!("{label} could not be read")))?;
    if !metadata.is_file() {
        return Err(CoreError::Config(format!("{label} is not a file")));
    }
    Ok(())
}

fn validate_local_path(
    storage: &infimount_mcp::registry::StorageRecord,
    path: &str,
) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path)
        .map_err(|_| CoreError::Config("Agent Task local path confinement check failed".into()))
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim().trim_matches('/');
    let relative = relative.trim().trim_matches('/');
    match (root.is_empty(), relative.is_empty()) {
        (true, true) => String::new(),
        (true, false) => relative.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}/{relative}"),
    }
}

fn create_neutral_handoff_directory() -> Result<PathBuf, CoreError> {
    let path = std::env::temp_dir().join(format!("infimount-codex-{}", Uuid::new_v4()));
    fs::create_dir(&path)
        .map_err(|_| CoreError::Config("failed to create the Codex handoff directory".into()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions_result = fs::metadata(&path).and_then(|metadata| {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions)
        });
        if permissions_result.is_err() {
            cleanup_neutral_directory(&path);
            return Err(CoreError::Config(
                "failed to secure the Codex handoff directory".into(),
            ));
        }
    }

    Ok(path)
}

fn cleanup_neutral_directory(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn codex_task_prompt(workspace_name: &str, task_path: &str, outputs_path: &str) -> String {
    format!(
        "Complete the prepared Infimount Agent Task using only the required task-scoped Infimount MCP server for task file access. Work only in Agent Workspace {workspace_name:?} at MCP path {task_path:?}. Read TASK.md through Infimount and treat inputs/ plus task metadata as immutable. Infimount enforces writes only under {outputs_path:?}; use write_file (with create_parents when needed) for deliverables. Stop when the deliverables are ready for review in Infimount. Do not publish outputs. Do not search for or use host filesystem paths to bypass the task boundary."
    )
}

fn codex_server_name(task_id: &str) -> String {
    format!("infimount_agent_task_{}", task_id.replace('-', ""))
}

fn push_codex_config(args: &mut Vec<String>, value: impl Into<String>) {
    args.push("-c".into());
    args.push(value.into());
}

fn codex_arguments(
    sidecar: &Path,
    cwd: &Path,
    prompt: &str,
    scope: &infimount_mcp::AgentTaskScope,
) -> Result<Vec<String>, CoreError> {
    let server_name = codex_server_name(&scope.task_id);
    let command = serde_json::to_string(&sidecar.to_string_lossy().to_string())?;
    let mcp_args = serde_json::to_string(&vec![
        "serve-agent-task".to_string(),
        "--task-id".to_string(),
        scope.task_id.clone(),
        "--storage-id".to_string(),
        scope.storage_id.clone(),
        "--workspace-id".to_string(),
        scope.workspace_id.clone(),
        "--policy-rule-id".to_string(),
        scope.policy_rule_id.clone(),
        "--storage-namespace-fingerprint".to_string(),
        scope.storage_namespace_fingerprint.clone(),
        "--workspace-prefix".to_string(),
        scope.workspace_prefix.clone(),
        "--task-prefix".to_string(),
        scope.task_prefix.clone(),
        "--outputs-prefix".to_string(),
        scope.outputs_prefix.clone(),
    ])?;

    let mut args = vec![
        "exec".into(),
        "--ephemeral".into(),
        "--ignore-user-config".into(),
        "--ignore-rules".into(),
        "--skip-git-repo-check".into(),
        "--sandbox".into(),
        "read-only".into(),
        "-C".into(),
        cwd.to_string_lossy().to_string(),
    ];

    // This run is intentionally MCP-only for task data access. Keep current Codex
    // authentication, but suppress ambient local execution, external integrations,
    // project instruction discovery, and skill catalogs. The task-unique MCP server
    // below is then the only file-write surface available to the model.
    for config in [
        "agents.enabled=false",
        "allow_login_shell=false",
        "check_for_update_on_startup=false",
        "include_apps_instructions=false",
        "include_collaboration_mode_instructions=false",
        "include_environment_context=false",
        "include_permissions_instructions=false",
        "project_doc_max_bytes=0",
        "skills.include_instructions=false",
        "skills.bundled.enabled=false",
        "features.shell_tool=false",
        "features.unified_exec=false",
        "features.shell_snapshot=false",
        "features.code_mode=false",
        "features.code_mode_host=false",
        "features.exec_permission_approvals=false",
        "features.request_permissions_tool=false",
        "features.hooks=false",
        "features.multi_agent=false",
        "features.multi_agent_v2=false",
        "features.view_image=false",
        "features.image_generation=false",
        "features.apps=false",
        "features.enable_mcp_apps=false",
        "features.tool_suggest=false",
        "features.recommended_plugins=false",
        "features.plugins=false",
        "features.executor_capability_discovery=false",
        "features.in_app_browser=false",
        "features.browser_use=false",
        "features.browser_use_full_cdp_access=false",
        "features.browser_use_external=false",
        "features.computer_use=false",
        "features.remote_plugin=false",
        "features.plugin_sharing=false",
        "features.skill_mcp_dependency_install=false",
        "features.skill_search=false",
        "features.artifact=false",
        "features.workspace_dependencies=false",
        "features.memories=false",
        "web_search=\"disabled\"",
    ] {
        push_codex_config(&mut args, config);
    }

    push_codex_config(
        &mut args,
        format!("mcp_servers.{server_name}.command={command}"),
    );
    push_codex_config(
        &mut args,
        format!("mcp_servers.{server_name}.args={mcp_args}"),
    );
    push_codex_config(
        &mut args,
        format!("mcp_servers.{server_name}.enabled=true"),
    );
    push_codex_config(
        &mut args,
        format!("mcp_servers.{server_name}.required=true"),
    );
    args.push(prompt.to_string());
    Ok(args)
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn render_posix_codex_command(args: &[String]) -> String {
    let rendered_args = args
        .iter()
        .map(|value| shell_quote(value))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "codex_bin=$(command -v codex 2>/dev/null || true); if [ -z \"$codex_bin\" ] || [ ! -f \"$codex_bin\" ] || [ ! -x \"$codex_bin\" ]; then printf '%s\\n' 'Codex CLI executable was not found in your login shell. Install or configure Codex, then retry from Infimount.'; \"${{SHELL:-/bin/sh}}\" -l; exit 127; fi; \"$codex_bin\" {rendered_args}"
    )
}

#[cfg(unix)]
fn write_posix_launcher(args: &[String], neutral_cwd: &Path) -> Result<PathBuf, CoreError> {
    use std::os::unix::fs::PermissionsExt;

    let script = neutral_cwd.join("launch.command");
    let command = render_posix_codex_command(args);
    let cleanup_dir = shell_quote(&neutral_cwd.to_string_lossy());
    let body = format!(
        "#!/bin/sh\nscript_path=$0\nrm -f -- \"$script_path\"\ncleanup() {{ rm -rf -- {cleanup_dir} 2>/dev/null || true; }}\ntrap cleanup EXIT HUP INT TERM\n\"${{SHELL:-/bin/sh}}\" -lic {}\nstatus=$?\ntrap - EXIT HUP INT TERM\ncleanup\nexit $status\n",
        shell_quote(&command)
    );
    if fs::write(&script, body).is_err() {
        return Err(CoreError::Config(
            "failed to prepare the Codex launcher".into(),
        ));
    }
    let permissions_result = fs::metadata(&script).and_then(|metadata| {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&script, permissions)
    });
    if permissions_result.is_err() {
        let _ = fs::remove_file(&script);
        return Err(CoreError::Config(
            "failed to prepare the Codex launcher".into(),
        ));
    }
    Ok(script)
}

#[cfg(target_os = "linux")]
fn launch_codex_terminal(args: &[String], neutral_cwd: &Path) -> Result<(), CoreError> {
    let script = match write_posix_launcher(args, neutral_cwd) {
        Ok(script) => script,
        Err(error) => {
            cleanup_neutral_directory(neutral_cwd);
            return Err(error);
        }
    };
    let candidates: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xterm", &["-e"]),
    ];
    for (terminal, prefix) in candidates {
        let mut command = Command::new(terminal);
        command.args(*prefix).arg(&script);
        match command.spawn() {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        }
    }
    cleanup_neutral_directory(neutral_cwd);
    Err(CoreError::Config(
        "No supported terminal application was found for Codex handoff".into(),
    ))
}

#[cfg(target_os = "macos")]
fn launch_codex_terminal(args: &[String], neutral_cwd: &Path) -> Result<(), CoreError> {
    let script = match write_posix_launcher(args, neutral_cwd) {
        Ok(script) => script,
        Err(error) => {
            cleanup_neutral_directory(neutral_cwd);
            return Err(error);
        }
    };
    match Command::new("open")
        .args(["-a", "Terminal"])
        .arg(&script)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(_) => {
            cleanup_neutral_directory(neutral_cwd);
            Err(CoreError::Config(
                "macOS Terminal could not be opened for Codex handoff".into(),
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn launch_codex_terminal(args: &[String], neutral_cwd: &Path) -> Result<(), CoreError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let script = neutral_cwd.join("launch.ps1");
    let rendered_args = args
        .iter()
        .map(|value| format!("'{}'", value.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let neutral = format!("'{}'", neutral_cwd.to_string_lossy().replace('\'', "''"));
    let body = format!(
        "$handoffDir = {neutral}\r\n$scriptPath = $PSCommandPath\r\ntry {{\r\n  Remove-Item -LiteralPath $scriptPath -Force -ErrorAction SilentlyContinue\r\n  $codex = Get-Command codex -ErrorAction SilentlyContinue\r\n  if (-not $codex -or $codex.CommandType -notin @('Application', 'ExternalScript')) {{ Write-Host 'Codex CLI executable was not found. Install or configure Codex, then retry from Infimount.'; return }}\r\n  $codexArgs = @({rendered_args})\r\n  & $codex.Source @codexArgs\r\n}} finally {{\r\n  Remove-Item -LiteralPath $handoffDir -Recurse -Force -ErrorAction SilentlyContinue\r\n}}\r\n"
    );
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(body.as_bytes());
    if fs::write(&script, bytes).is_err() {
        cleanup_neutral_directory(neutral_cwd);
        return Err(CoreError::Config(
            "failed to prepare the Codex launcher".into(),
        ));
    }
    match Command::new("powershell.exe")
        .args(["-NoExit", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(_) => {
            cleanup_neutral_directory(neutral_cwd);
            Err(CoreError::Config(
                "PowerShell could not be opened for Codex handoff".into(),
            ))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn launch_codex_terminal(_args: &[String], neutral_cwd: &Path) -> Result<(), CoreError> {
    cleanup_neutral_directory(neutral_cwd);
    Err(CoreError::Config(
        "Codex handoff is not supported on this operating system".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};

    fn test_scope() -> infimount_mcp::AgentTaskScope {
        infimount_mcp::AgentTaskScope::new(
            "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            "storage-id".into(),
            "Workspace storage".into(),
            "workspace-id".into(),
            "workspace:workspace-id".into(),
            "namespace-fingerprint".into(),
            "agent/research".into(),
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454/outputs".into(),
        )
        .unwrap()
    }

    fn test_manifest() -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            title: "Test task".into(),
            created_at: "2026-09-08T00:00:00Z".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_root: "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            outputs_directory: "outputs".into(),
            inputs: vec![
                AgentTaskInput {
                    task_path: "inputs/a.txt".into(),
                    byte_size: 1,
                    sha256: "a".repeat(64),
                },
                AgentTaskInput {
                    task_path: "inputs/nested/b.txt".into(),
                    byte_size: 2,
                    sha256: "b".repeat(64),
                },
            ],
        }
    }

    #[test]
    fn prepared_input_paths_are_derived_only_from_manifest_entries() {
        let paths = expected_prepared_input_paths(
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454",
            &test_manifest(),
        );
        assert_eq!(paths.len(), 2);
        assert!(paths
            .contains("agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454/inputs/a.txt"));
        assert!(paths.contains(
            "agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454/inputs/nested/b.txt"
        ));
    }

    #[test]
    fn codex_prompt_uses_mcp_scope_without_host_path() {
        let prompt = codex_task_prompt(
            "Research",
            "/Workspace storage/agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454",
            "/Workspace storage/agent/research/tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454/outputs",
        );
        assert!(prompt.contains("task-scoped Infimount MCP server"));
        assert!(prompt.contains("writes only under"));
        assert!(prompt.contains("Do not publish outputs"));
        assert!(!prompt.contains("/home/"));
        assert!(!prompt.contains("C:\\"));
    }

    #[test]
    fn codex_arguments_are_ephemeral_isolated_and_task_scoped() {
        let scope = test_scope();
        let args = codex_arguments(
            Path::new("/opt/Infimount/mcp"),
            Path::new("/tmp/neutral"),
            "do the task",
            &scope,
        )
        .unwrap();
        let joined = args.join(" ");
        let server_name = codex_server_name(&scope.task_id);
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(joined.contains("--ephemeral"));
        assert!(joined.contains("--ignore-user-config"));
        assert!(joined.contains("--ignore-rules"));
        assert!(joined.contains("--skip-git-repo-check"));
        assert!(joined.contains("--sandbox read-only"));
        assert!(joined.contains("agents.enabled=false"));
        assert!(joined.contains("project_doc_max_bytes=0"));
        assert!(joined.contains("skills.include_instructions=false"));
        assert!(joined.contains("skills.bundled.enabled=false"));
        assert!(joined.contains("features.shell_tool=false"));
        assert!(joined.contains("features.unified_exec=false"));
        assert!(joined.contains("features.code_mode_host=false"));
        assert!(joined.contains("features.hooks=false"));
        assert!(joined.contains("features.multi_agent=false"));
        assert!(joined.contains("features.browser_use=false"));
        assert!(joined.contains("features.computer_use=false"));
        assert!(joined.contains("features.plugins=false"));
        assert!(joined.contains("features.workspace_dependencies=false"));
        assert!(joined.contains("features.view_image=false"));
        assert!(joined.contains("web_search=\"disabled\""));
        assert!(joined.contains(&format!("mcp_servers.{server_name}.command")));
        assert!(joined.contains(&format!("mcp_servers.{server_name}.required=true")));
        assert!(joined.contains("serve-agent-task"));
        assert!(joined.contains("--storage-id"));
        assert!(joined.contains("--workspace-id"));
        assert!(joined.contains("--policy-rule-id"));
        assert!(joined.contains("--storage-namespace-fingerprint"));
        assert!(joined.contains("--task-prefix"));
        assert!(joined.contains("--outputs-prefix"));
        assert!(!joined.contains("--transport"));
        assert!(!joined.contains("approval_policy"));
        assert!(!joined.contains("dangerously"));
    }

    #[test]
    fn codex_server_name_is_unique_per_task_and_safe_for_dotted_config() {
        let name = codex_server_name("81f08176-86e4-40ec-a9a4-a219c4c9b454");
        assert_eq!(
            name,
            "infimount_agent_task_81f0817686e440eca9a4a219c4c9b454"
        );
        assert!(name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
    }

    #[cfg(unix)]
    #[test]
    fn posix_launcher_quotes_every_codex_argument_and_keeps_cleanup_owner() {
        let rendered = render_posix_codex_command(&[
            "exec".into(),
            "-C".into(),
            "/tmp/a path".into(),
            "prompt with ' quote".into(),
        ]);
        assert!(rendered.contains("\"$codex_bin\""));
        assert!(rendered.contains("'/tmp/a path'"));
        assert!(rendered.contains("'prompt with '\"'\"' quote'"));
        assert!(!rendered.contains("exec codex"));

        let temp = tempfile::tempdir().unwrap();
        let handoff = temp.path().join("handoff");
        fs::create_dir(&handoff).unwrap();
        let launcher = write_posix_launcher(&["prompt".into()], &handoff).unwrap();
        let body = fs::read_to_string(launcher).unwrap();
        assert!(body.contains("trap cleanup EXIT HUP INT TERM"));
        assert!(!body.contains("exec \"${SHELL:-/bin/sh}\" -lic"));
    }
}
