use std::collections::HashSet;
use std::time::Instant;

use infimount_core::SourceKind;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResult, ErrorData, Implementation,
    ListToolsResult, ServerCapabilities, ServerInfo, Tool, ToolsCapability,
};
use rmcp::{ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};

use crate::audit::{AuditDecision, AuditEvent, AuditStore};
use crate::errors::{err, err_with_details, fail, McpError, McpErrorCode, McpResult};
use crate::path::parse_mcp_path;
use crate::policy::{normalize_policy_path, McpAccessMode, McpOperation, McpRuleSource};
use crate::registry::{StorageRecord, StorageRegistry};
use crate::server::{self, rmcp_tools};
use crate::session::SessionManager;
use crate::storage_namespace::storage_namespace_fingerprint;
use crate::tools_fs::FsToolsContext;

const AGENT_TASK_TOOL_NAMES: &[&str] = &[
    "list_dir",
    "stat_path",
    "read_file",
    "search_paths",
    "write_file",
    "mkdir",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTaskScope {
    pub task_id: String,
    pub storage_id: String,
    pub storage_name: String,
    pub workspace_id: String,
    pub policy_rule_id: String,
    pub storage_namespace_fingerprint: String,
    pub workspace_prefix: String,
    pub task_prefix: String,
    pub outputs_prefix: String,
}

impl AgentTaskScope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: String,
        storage_id: String,
        storage_name: String,
        workspace_id: String,
        policy_rule_id: String,
        storage_namespace_fingerprint: String,
        workspace_prefix: String,
        task_prefix: String,
        outputs_prefix: String,
    ) -> McpResult<Self> {
        for (label, value) in [
            ("task id", task_id.as_str()),
            ("storage id", storage_id.as_str()),
            ("storage name", storage_name.as_str()),
            ("workspace id", workspace_id.as_str()),
            ("policy rule id", policy_rule_id.as_str()),
            (
                "storage namespace fingerprint",
                storage_namespace_fingerprint.as_str(),
            ),
        ] {
            if value.trim().is_empty() || value.chars().any(char::is_control) {
                return Err(err(
                    McpErrorCode::ERR_INVALID_PATH,
                    format!("Agent Task {label} is invalid"),
                ));
            }
        }

        let workspace_prefix = normalize_non_root_prefix(&workspace_prefix, "workspace")?;
        let task_prefix = normalize_non_root_prefix(&task_prefix, "task")?;
        let outputs_prefix = normalize_non_root_prefix(&outputs_prefix, "outputs")?;
        if !path_matches_prefix(&task_prefix, &workspace_prefix) || task_prefix == workspace_prefix {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "Agent Task scope must be below its Agent Workspace",
            ));
        }
        if !path_matches_prefix(&outputs_prefix, &task_prefix) || outputs_prefix == task_prefix {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "Agent Task outputs scope must be below the task root",
            ));
        }

        Ok(Self {
            task_id,
            storage_id,
            storage_name,
            workspace_id,
            policy_rule_id,
            storage_namespace_fingerprint,
            workspace_prefix,
            task_prefix,
            outputs_prefix,
        })
    }

    pub fn validate_current_binding(&self, registry: &StorageRegistry) -> McpResult<StorageRecord> {
        let storage = registry
            .load_all()?
            .into_iter()
            .find(|storage| storage.id == self.storage_id)
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    "Agent Task storage is no longer available",
                )
            })?;
        if storage.name != self.storage_name {
            return Err(err(
                McpErrorCode::ERR_STORAGE_NAMESPACE_IN_USE,
                "Agent Task storage identity changed after handoff",
            ));
        }
        if !storage.enabled {
            return Err(err(
                McpErrorCode::ERR_STORAGE_DISABLED,
                "Agent Task storage is disabled",
            ));
        }
        if !storage.mcp_exposed {
            return Err(err(
                McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
                "Agent Task storage is no longer exposed to MCP",
            ));
        }
        if storage.read_only {
            return Err(err(
                McpErrorCode::ERR_STORAGE_READ_ONLY,
                "Agent Task storage is read-only",
            ));
        }
        let kind = storage.backend.parse::<SourceKind>().map_err(|_| {
            err(
                McpErrorCode::ERR_BACKEND_UNSUPPORTED,
                "Agent Task storage backend is unsupported",
            )
        })?;
        if !matches!(kind, SourceKind::Local) {
            return Err(err(
                McpErrorCode::ERR_BACKEND_UNSUPPORTED,
                "Codex Agent Task handoff currently requires Local Filesystem storage",
            ));
        }

        let namespace = storage_namespace_fingerprint(&storage)?;
        if namespace != self.storage_namespace_fingerprint {
            return Err(err(
                McpErrorCode::ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED,
                "Agent Task storage namespace changed after handoff",
            ));
        }

        let rule = storage
            .mcp_policy
            .rules
            .iter()
            .find(|rule| rule.id == self.policy_rule_id)
            .ok_or_else(|| {
                err(
                    McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
                    "Agent Workspace MCP policy grant is missing",
                )
            })?;
        let rule_prefix = normalize_policy_path(&rule.prefix)?;
        let source_matches = matches!(
            &rule.source,
            McpRuleSource::Workspace { workspace_id } if workspace_id == &self.workspace_id
        );
        if rule_prefix != self.workspace_prefix
            || rule.access != McpAccessMode::ReadWrite
            || !source_matches
        {
            return Err(err(
                McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
                "Agent Workspace MCP policy grant no longer matches this workspace",
            ));
        }

        Ok(storage)
    }

    fn authorize_path(
        &self,
        registry: &StorageRegistry,
        path: &str,
        access: ScopeAccess,
    ) -> McpResult<(StorageRecord, String)> {
        let storage = self.validate_current_binding(registry)?;
        let parsed = parse_mcp_path(path)?;
        if parsed.is_root {
            return Err(err(
                McpErrorCode::ERR_SESSION_FORBIDDEN,
                "Agent Task MCP does not expose the Infimount root",
            ));
        }
        if parsed.storage_name.as_deref() != Some(self.storage_name.as_str()) {
            return Err(err(
                McpErrorCode::ERR_SESSION_FORBIDDEN,
                "path is outside the prepared Agent Task storage",
            ));
        }
        let backend_path = normalize_policy_path(&parsed.backend_path)?;
        let allowed_prefix = match access {
            ScopeAccess::Read => &self.task_prefix,
            ScopeAccess::Write => &self.outputs_prefix,
        };
        if !path_matches_prefix(&backend_path, allowed_prefix) {
            return Err(err_with_details(
                McpErrorCode::ERR_SESSION_FORBIDDEN,
                match access {
                    ScopeAccess::Read => "path is outside the prepared Agent Task",
                    ScopeAccess::Write => "Agent Task writes are restricted to outputs/",
                },
                json!({ "taskId": self.task_id }),
            ));
        }
        Ok((storage, parsed.normalized))
    }
}

#[derive(Debug, Clone, Copy)]
enum ScopeAccess {
    Read,
    Write,
}

fn normalize_non_root_prefix(value: &str, label: &str) -> McpResult<String> {
    let normalized = normalize_policy_path(value)?;
    if normalized.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("Agent Task {label} scope must not be the storage root"),
        ));
    }
    Ok(normalized)
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub struct AgentTaskMcpServer {
    ctx: FsToolsContext,
    scope: AgentTaskScope,
    enabled_tools: HashSet<String>,
    audit: AuditStore,
}

impl AgentTaskMcpServer {
    pub fn new(
        registry: StorageRegistry,
        enabled_tools: Vec<String>,
        scope: AgentTaskScope,
    ) -> McpResult<Self> {
        scope.validate_current_binding(&registry)?;
        let allowed = AGENT_TASK_TOOL_NAMES
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        let enabled_tools = enabled_tools
            .into_iter()
            .filter(|name| allowed.contains(name.as_str()))
            .collect::<HashSet<_>>();
        if !enabled_tools.contains("write_file") {
            return Err(err(
                McpErrorCode::ERR_MCP_POLICY_DENIED,
                "Codex Agent Task handoff requires the write_file MCP tool to be enabled",
            ));
        }
        for required in ["list_dir", "stat_path", "read_file"] {
            if !enabled_tools.contains(required) {
                return Err(err(
                    McpErrorCode::ERR_MCP_POLICY_DENIED,
                    format!("Codex Agent Task handoff requires the {required} MCP tool to be enabled"),
                ));
            }
        }
        Ok(Self {
            ctx: FsToolsContext {
                registry,
                sessions: SessionManager::new(),
                allow_insecure: true,
                auth_token: None,
            },
            scope,
            enabled_tools,
            audit: AuditStore::new(None),
        })
    }

    fn is_tool_enabled(&self, name: &str) -> bool {
        self.enabled_tools.contains(name)
    }

    fn tools(&self) -> Vec<Tool> {
        rmcp_tools()
            .into_iter()
            .filter(|tool| self.enabled_tools.contains(tool.name.as_ref()))
            .collect()
    }

    async fn dispatch_tool(&self, name: &str, arguments: Map<String, Value>) -> Value {
        let raw = Value::Object(arguments);
        let Some(path) = raw.get("path").and_then(Value::as_str) else {
            return error_value(err(
                McpErrorCode::ERR_INVALID_PATH,
                "Agent Task file tools require a path",
            ));
        };
        let (access, operation) = match name {
            "list_dir" => (ScopeAccess::Read, McpOperation::List),
            "stat_path" => (ScopeAccess::Read, McpOperation::Metadata),
            "read_file" => (ScopeAccess::Read, McpOperation::Read),
            "search_paths" => (ScopeAccess::Read, McpOperation::Search),
            "write_file" => (ScopeAccess::Write, McpOperation::Write),
            "mkdir" => (ScopeAccess::Write, McpOperation::Mkdir),
            _ => return error_value(err(McpErrorCode::ERR_INVALID_PATH, "unsupported Agent Task tool")),
        };

        let started = Instant::now();
        let configuration = match self.ctx.registry.acquire_configuration_transaction() {
            Ok(lock) => lock,
            Err(error) => return error_value(error),
        };
        let authorization = self.scope.authorize_path(&self.ctx.registry, path, access);
        let (storage, normalized_path) = match authorization {
            Ok(value) => value,
            Err(error) => {
                drop(configuration);
                let result = error_value(error);
                self.audit_result(name, operation, path, None, &result, started.elapsed().as_millis() as u64);
                return result;
            }
        };

        // Calling the typed filesystem handlers directly is intentional here. The user's
        // explicit Open in Codex action authorizes non-destructive writes only under the
        // already-prepared outputs/ prefix. The normal storage policy is still evaluated by
        // every filesystem handler; destructive and publishing tools are not exposed at all.
        let result = match name {
            "list_dir" => server::invoke_list_dir_json(&self.ctx, raw).await,
            "stat_path" => server::invoke_stat_path_json(&self.ctx, raw).await,
            "read_file" => server::invoke_read_file_json(&self.ctx, raw).await,
            "search_paths" => server::invoke_search_paths_json(&self.ctx, raw).await,
            "write_file" => server::invoke_write_file_json(&self.ctx, raw).await,
            "mkdir" => server::invoke_mkdir_json(&self.ctx, raw).await,
            _ => unreachable!("tool checked above"),
        };
        drop(configuration);
        self.audit_result(
            name,
            operation,
            &normalized_path,
            Some(&storage),
            &result,
            started.elapsed().as_millis() as u64,
        );
        result
    }

    fn audit_result(
        &self,
        name: &str,
        operation: McpOperation,
        path: &str,
        storage: Option<&StorageRecord>,
        result: &Value,
        duration_ms: u64,
    ) {
        let mut event = AuditEvent::new(name, operation);
        event.mcp_client_id = Some("codex-agent-task".to_string());
        event.path = Some(path.to_string());
        event.workspace_id = Some(self.scope.workspace_id.clone());
        event.matched_rule_id = Some(self.scope.policy_rule_id.clone());
        event.duration_ms = Some(duration_ms);
        if let Some(storage) = storage {
            event.storage_id = Some(storage.id.clone());
            event.storage_name = Some(storage.name.clone());
            event.backend = Some(storage.backend.clone());
        } else {
            event.storage_id = Some(self.scope.storage_id.clone());
            event.storage_name = Some(self.scope.storage_name.clone());
        }
        let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if ok {
            event.decision = AuditDecision::Allowed;
            if name == "write_file" {
                event.bytes_written = result
                    .get("data")
                    .and_then(|data| data.get("written_bytes"))
                    .and_then(Value::as_u64);
            }
        } else {
            let code = result
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("ERR_INTERNAL");
            event.error_code = Some(code.to_string());
            event.decision = if matches!(
                code,
                "ERR_MCP_POLICY_DENIED"
                    | "ERR_STORAGE_DISABLED"
                    | "ERR_STORAGE_NOT_EXPOSED"
                    | "ERR_STORAGE_READ_ONLY"
                    | "ERR_SESSION_FORBIDDEN"
            ) {
                AuditDecision::Denied
            } else {
                AuditDecision::Failed
            };
        }
        let _ = self.audit.append(event);
    }
}

impl ServerHandler for AgentTaskMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools_with(ToolsCapability {
                    list_changed: Some(false),
                })
                .build(),
        )
        .with_server_info(
            Implementation::new("infimount_agent_task", env!("CARGO_PKG_VERSION"))
                .with_title("Infimount Agent Task MCP Server")
                .with_description("Task-scoped filesystem access for a prepared Infimount Agent Task."),
        )
        .with_instructions(
            "Use only the prepared Agent Task path. Inputs and task instructions are readable; writes are restricted to outputs/. Infimount root enumeration, destructive operations, and publication are unavailable.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = request.name.to_string();
        if !self.is_tool_enabled(&name) {
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        }
        let result = self
            .dispatch_tool(&name, request.arguments.unwrap_or_default())
            .await;
        let is_error = result
            .get("ok")
            .and_then(Value::as_bool)
            .map(|ok| !ok)
            .unwrap_or(true);
        if is_error {
            Ok(CallToolResult::structured_error(result))
        } else {
            Ok(CallToolResult::structured(result))
        }
    }
}

fn error_value(error: McpError) -> Value {
    serde_json::to_value(fail(error)).unwrap_or_else(|_| {
        json!({
            "ok": false,
            "error": {
                "code": "ERR_INTERNAL",
                "message": "failed to serialize Agent Task MCP error",
                "details": {}
            }
        })
    })
}

pub async fn serve_agent_task_stdio(
    registry: StorageRegistry,
    enabled_tools: Vec<String>,
    scope: AgentTaskScope,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = AgentTaskMcpServer::new(registry, enabled_tools, scope)
        .map_err(|error| std::io::Error::other(error.message))?;
    let (stdin, stdout) = rmcp::transport::stdio();
    let running = service
        .serve((stdin, stdout))
        .await
        .map_err(rmcp::RmcpError::from)?;
    running.waiting().await.map_err(rmcp::RmcpError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{McpConfirmationRules, McpPathRule, McpStoragePolicy};
    use serde_json::json;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, StorageRegistry, AgentTaskScope) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("workspace/tasks/task-1/outputs")).unwrap();
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        let mut storage = StorageRecord::new(
            "Workspace storage".to_string(),
            "local".to_string(),
            json!({ "root": root }),
        );
        storage.id = "storage-1".to_string();
        storage.mcp_exposed = true;
        storage.mcp_policy = McpStoragePolicy {
            default_access: McpAccessMode::None,
            rules: vec![McpPathRule {
                id: "workspace:workspace-1".to_string(),
                prefix: "workspace".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Workspace {
                    workspace_id: "workspace-1".to_string(),
                },
                confirmation_rules: Some(McpConfirmationRules::default()),
            }],
            ..Default::default()
        };
        let namespace = storage_namespace_fingerprint(&storage).unwrap();
        registry.save_all_atomic(&[storage]).unwrap();
        let scope = AgentTaskScope::new(
            "task-1".to_string(),
            "storage-1".to_string(),
            "Workspace storage".to_string(),
            "workspace-1".to_string(),
            "workspace:workspace-1".to_string(),
            namespace,
            "workspace".to_string(),
            "workspace/tasks/task-1".to_string(),
            "workspace/tasks/task-1/outputs".to_string(),
        )
        .unwrap();
        (dir, registry, scope)
    }

    #[test]
    fn scope_allows_reads_only_inside_task_and_writes_only_inside_outputs() {
        let (_dir, registry, scope) = fixture();
        assert!(scope
            .authorize_path(
                &registry,
                "/Workspace storage/workspace/tasks/task-1/TASK.md",
                ScopeAccess::Read,
            )
            .is_ok());
        assert!(scope
            .authorize_path(
                &registry,
                "/Workspace storage/workspace/tasks/task-1/inputs/a.txt",
                ScopeAccess::Write,
            )
            .is_err());
        assert!(scope
            .authorize_path(
                &registry,
                "/Workspace storage/workspace/tasks/task-1/outputs/result.md",
                ScopeAccess::Write,
            )
            .is_ok());
        assert!(scope
            .authorize_path(
                &registry,
                "/Workspace storage/workspace/tasks/task-2/outputs/result.md",
                ScopeAccess::Read,
            )
            .is_err());
        assert!(scope
            .authorize_path(
                &registry,
                "/Other/workspace/tasks/task-1/TASK.md",
                ScopeAccess::Read,
            )
            .is_err());
        assert!(scope
            .authorize_path(&registry, "/", ScopeAccess::Read)
            .is_err());
    }

    #[test]
    fn scope_fails_closed_when_workspace_policy_binding_changes() {
        let (_dir, registry, scope) = fixture();
        let mut storage = registry.load_all().unwrap().remove(0);
        storage.mcp_policy.rules[0].source = McpRuleSource::Manual;
        registry.save_all_atomic(&[storage]).unwrap();
        assert!(scope.validate_current_binding(&registry).is_err());
    }

    #[test]
    fn task_server_filters_global_tools_to_non_destructive_task_surface() {
        let (_dir, registry, scope) = fixture();
        let server = AgentTaskMcpServer::new(
            registry,
            vec![
                "list_dir".into(),
                "stat_path".into(),
                "read_file".into(),
                "search_paths".into(),
                "write_file".into(),
                "mkdir".into(),
                "delete_path".into(),
                "move_path".into(),
                "generate_download_link".into(),
            ],
            scope,
        )
        .unwrap();
        assert!(server.is_tool_enabled("write_file"));
        assert!(!server.is_tool_enabled("delete_path"));
        assert!(!server.is_tool_enabled("move_path"));
        assert!(!server.is_tool_enabled("generate_download_link"));
    }
}
