//! A4 Claude Code security contract.
//! Intentionally NOT callable from the UI until real packaged testing proves
//! these flags confine that specific Claude Code build to task-scoped MCP.
use infimount_core::CoreError;
use infimount_mcp::AgentTaskScope;
use serde_json::{json, Map, Value};
use std::path::Path;

const MIN_CLAUDE_VERSION: (u64, u64, u64) = (2, 1, 259);
const TOOLS: [&str; 6] = [
    "list_dir",
    "stat_path",
    "read_file",
    "search_paths",
    "write_file",
    "mkdir",
];
const DENIED_HOST_TOOLS: [&str; 12] = [
    "Bash",
    "Read",
    "Edit",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Agent",
    "Task",
    "Skill",
    "TaskCreate",
    "TaskUpdate",
];

fn invalid(message: &'static str) -> CoreError {
    CoreError::Config(message.into())
}

fn parse_claude_version(value: &str) -> Result<(u64, u64, u64), CoreError> {
    let raw = value.trim();
    if raw.is_empty()
        || raw.len() > 100
        || !raw.is_ascii()
        || raw.chars().any(char::is_control)
        || raw.contains('/')
        || raw.contains('\\')
    {
        return Err(invalid("Claude Code version evidence is invalid"));
    }
    let (version, suffix) = raw.split_once(' ').unwrap_or((raw, ""));
    if !suffix.is_empty() && suffix != "(Claude Code)" {
        return Err(invalid("Claude Code version output was not recognized"));
    }
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid("Claude Code version must be X.Y.Z"));
    }
    Ok((
        parts[0]
            .parse::<u64>()
            .map_err(|_| invalid("invalid Claude Code version"))?,
        parts[1]
            .parse::<u64>()
            .map_err(|_| invalid("invalid Claude Code version"))?,
        parts[2]
            .parse::<u64>()
            .map_err(|_| invalid("invalid Claude Code version"))?,
    ))
}

fn require_qualified_flags(version: &str) -> Result<(), CoreError> {
    if parse_claude_version(version)? < MIN_CLAUDE_VERSION {
        return Err(invalid(
            "Claude Code lacks required unattended restriction flags",
        ));
    }
    Ok(())
}

fn task_mcp_server_name(scope: &AgentTaskScope) -> Result<String, CoreError> {
    let id = &scope.task_id;
    if uuid::Uuid::parse_str(id).is_err()
        || id.len() != 36
        || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err(invalid("Agent Task scope contains an invalid task ID"));
    }
    Ok(format!("infimount_agent_task_{}", id.replace('-', "")))
}

fn task_server_args(scope: &AgentTaskScope) -> Vec<String> {
    vec![
        "serve-agent-task".into(),
        "--task-id".into(),
        scope.task_id.clone(),
        "--storage-id".into(),
        scope.storage_id.clone(),
        "--workspace-id".into(),
        scope.workspace_id.clone(),
        "--policy-rule-id".into(),
        scope.policy_rule_id.clone(),
        "--storage-namespace-fingerprint".into(),
        scope.storage_namespace_fingerprint.clone(),
        "--workspace-prefix".into(),
        scope.workspace_prefix.clone(),
        "--task-prefix".into(),
        scope.task_prefix.clone(),
        "--outputs-prefix".into(),
        scope.outputs_prefix.clone(),
    ]
}

/// Draft, non-executable plan for a fail-closed MCP-only session. Bare mode
/// skips discovered configuration, restricted mode reduces host capabilities,
/// --tools "" removes built-ins but does not remove MCP tools, and dontAsk plus
/// permission-prompts none rejects unattended permission requests.
pub(crate) fn claude_argv(
    sidecar: &Path,
    scope: &AgentTaskScope,
    task_prompt: &str,
    reported_version: &str,
) -> Result<Vec<String>, CoreError> {
    require_qualified_flags(reported_version)?;
    if !sidecar.is_absolute() {
        return Err(invalid("Claude Code requires an absolute verified sidecar"));
    }
    if task_prompt.is_empty() || task_prompt.len() > 32768 {
        return Err(invalid("Claude Code task prompt is empty or too long"));
    }
    let command = sidecar
        .to_str()
        .ok_or_else(|| invalid("MCP sidecar path is not UTF-8"))?;
    let server = task_mcp_server_name(scope)?;
    let mut servers = Map::new();
    servers.insert(
        server.clone(),
        json!({
            "command": command,
            "args": task_server_args(scope),
            "env": {},
        }),
    );
    let mcp_config = serde_json::to_string(&json!({"mcpServers": Value::Object(servers)}))?;
    let settings = serde_json::to_string(&json!({
        "permissions": {"defaultMode": "dontAsk", "deny": DENIED_HOST_TOOLS}
    }))?;
    let mut args = vec![
        "--bare".into(),
        "--restricted".into(),
        "--no-chrome".into(),
        "--disable-slash-commands".into(),
        "--no-session-persistence".into(),
        "--strict-mcp-config".into(),
        "--permission-mode".into(),
        "dontAsk".into(),
        "--permission-prompts".into(),
        "none".into(),
        "--tools".into(),
        "".into(),
        "--max-turns".into(),
        "24".into(),
        "--output-format".into(),
        "json".into(),
        "--settings".into(),
        settings,
        "--mcp-config".into(),
        mcp_config,
        "--allowedTools".into(),
    ];
    for tool in TOOLS {
        args.push(format!("mcp__{server}__{tool}"));
    }
    args.push("--print".into());
    args.push(task_prompt.into());
    validate_argv(&args, &server)?;
    Ok(args)
}

/// Validate our fixed launch profile defensively. No arbitrary client-supplied
/// tools, unmanaged MCP server configurations or host execution may be added.
fn validate_argv(args: &[String], server: &str) -> Result<(), CoreError> {
    for flag in [
        "--bare",
        "--restricted",
        "--no-chrome",
        "--disable-slash-commands",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--print",
    ] {
        if args.iter().filter(|arg| arg.as_str() == flag).count() != 1 {
            return Err(invalid("Claude Code confinement flag missing or repeated"));
        }
    }
    for (flag, value) in [
        ("--permission-mode", "dontAsk"),
        ("--permission-prompts", "none"),
        ("--tools", ""),
        ("--max-turns", "24"),
        ("--output-format", "json"),
    ] {
        if args
            .windows(2)
            .filter(|p| p[0] == flag && p[1] == value)
            .count()
            != 1
        {
            return Err(invalid("Claude Code confinement option invalid"));
        }
    }
    let config = args
        .windows(2)
        .find(|pair| pair[0] == "--mcp-config")
        .ok_or_else(|| invalid("Claude MCP configuration missing"))?;
    let parsed: Value = serde_json::from_str(&config[1])?;
    let servers = parsed["mcpServers"]
        .as_object()
        .ok_or_else(|| invalid("invalid MCP servers"))?;
    if servers.len() != 1 || !servers.contains_key(server) {
        return Err(invalid("Claude Code must use exactly one task MCP server"));
    }
    let tools = TOOLS
        .iter()
        .map(|t| format!("mcp__{server}__{t}"))
        .collect::<Vec<_>>();
    let allowed = args
        .iter()
        .position(|a| a == "--allowedTools")
        .ok_or_else(|| invalid("Claude Code MCP allowlist missing"))?;
    let print_at = allowed + 1 + tools.len();
    if args.get(allowed + 1..print_at) != Some(tools.as_slice())
        || args.get(print_at).map(String::as_str) != Some("--print")
        || args.len() != print_at + 2
    {
        return Err(invalid(
            "Claude Code MCP allowlist or argument tail differs from task scope",
        ));
    }
    if !matches!(args.last(), Some(prompt) if !prompt.is_empty()) {
        return Err(invalid("Claude Code prompt was dropped"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> AgentTaskScope {
        AgentTaskScope::new(
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
    fn good_args() -> Vec<String> {
        claude_argv(
            Path::new("/opt/Infimount/mcp"),
            &scope(),
            "Do the bounded task using task-scoped MCP only",
            "2.1.259 (Claude Code)",
        )
        .unwrap()
    }

    #[test]
    fn refuses_old_or_untrusted_client_versions() {
        assert!(require_qualified_flags("2.1.259 (Claude Code)").is_ok());
        assert!(require_qualified_flags("2.2.0").is_ok());
        for version in [
            "",
            "2.1.258",
            "1.9.0",
            "foo",
            "2.1.259\nsecret",
            "2.1.259 /secret",
            "2.1.259 not-Claude",
        ] {
            assert!(require_qualified_flags(version).is_err(), "{version:?}");
        }
    }

    #[test]
    fn flags_restrict_builtin_tools_and_unattended_approvals() {
        let args = good_args();
        let name = task_mcp_server_name(&scope()).unwrap();
        validate_argv(&args, &name).unwrap();
        assert!(args
            .windows(2)
            .any(|p| p[0] == "--tools" && p[1].is_empty()));
        assert!(args
            .windows(2)
            .any(|p| p[0] == "--permission-prompts" && p[1] == "none"));
        assert!(!args
            .iter()
            .any(|s| s == "--dangerously-skip-permissions" || s == "bypassPermissions"));
    }

    #[test]
    fn sole_mcp_server_exposes_exactly_six_task_tools() {
        let args = good_args();
        let config = args
            .windows(2)
            .find(|pair| pair[0] == "--mcp-config")
            .unwrap();
        let data: Value = serde_json::from_str(&config[1]).unwrap();
        assert_eq!(data["mcpServers"].as_object().unwrap().len(), 1);
        let name = task_mcp_server_name(&scope()).unwrap();
        assert_eq!(data["mcpServers"][&name]["command"], "/opt/Infimount/mcp");
        let allowed = args.iter().filter(|s| s.starts_with("mcp__")).count();
        assert_eq!(allowed, 6);
        let joined = args.join(" ");
        assert!(!joined.contains("/home/"));
        assert!(!joined.contains("Bearer "));
    }

    #[test]
    fn altered_tool_scope_fails_closed() {
        let name = task_mcp_server_name(&scope()).unwrap();
        let mut args = good_args();
        args.retain(|a| !a.ends_with("__write_file"));
        assert!(validate_argv(&args, &name).is_err());

        let mut args = good_args();
        args.push("mcp__other__delete_path".into());
        assert!(validate_argv(&args, &name).is_err());

        let mut args = good_args();
        let pos = args.iter().position(|arg| arg == "--allowedTools").unwrap();
        args.insert(pos + 1, "mcp__other__read_file".into());
        assert!(validate_argv(&args, &name).is_err());
    }

    #[test]
    fn rejects_relative_sidecar_and_unbounded_prompts() {
        assert!(claude_argv(Path::new("relative/mcp"), &scope(), "prompt", "2.1.259").is_err());
        assert!(claude_argv(Path::new("/opt/mcp"), &scope(), "", "2.1.259").is_err());
        assert!(claude_argv(
            Path::new("/opt/mcp"),
            &scope(),
            &"x".repeat(32769),
            "2.1.259"
        )
        .is_err());
    }

    #[test]
    fn scoped_sidecar_args_match_codex_task_transport() {
        let args = task_server_args(&scope()).join(" ");
        assert!(args.contains("--task-prefix"));
        assert!(args.contains("--outputs-prefix"));
        assert!(!args.contains("/home/"));
        assert!(!args.contains("C:"));
    }
}
