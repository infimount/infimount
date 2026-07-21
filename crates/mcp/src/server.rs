use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResult, ErrorData,
    GetPromptRequestParams, GetPromptResult, Implementation, JsonObject, ListPromptsResult,
    ListResourcesRequestMethod, ListResourcesResult, ListToolsResult, PromptsCapability,
    ReadResourceRequestMethod, ReadResourceRequestParams, ReadResourceResult, ResourcesCapability,
    ServerCapabilities, ServerInfo, Tool, ToolAnnotations, ToolsCapability,
};
use rmcp::ServerHandler;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::audit::{AuditDecision, AuditEvent, AuditStore};
use crate::confirmation::{ConfirmationManager, ConfirmationRequest, ConfirmationRequiredResponse};
use crate::errors::{err_with_details, wrap_json, McpErrorCode, McpResult};
use crate::policy::{evaluate_storage_policy, McpOperation, McpRiskType, PolicyDecision};
use crate::prompts;
use crate::resources;
use crate::schemas;
use crate::session::{SessionCreateInput, SessionCreateOutput, SessionEndInput, SessionEndOutput};
use crate::telemetry::TelemetryState;
#[allow(unused_imports)]
use crate::tools_fs::{
    self, CopyPathInput, CopyPathOutput, DeletePathInput, DeletePathOutput, DeleteVersionInput,
    DeleteVersionOutput, FsToolsContext, GenerateDownloadLinkInput, GenerateDownloadLinkOutput,
    ListDirInput, ListDirOutput, ListVersionsInput, MkdirInput, MkdirOutput, MovePathInput,
    MovePathOutput, ReadFileInput, ReadFileOutput, ReadFileVersionInput, SearchPathsInput,
    SearchPathsOutput, StatPathInput, StatPathOutput, WriteFileInput, WriteFileOutput,
};
// tools_storage functions remain available for desktop-internal calls
use crate::tools_storage::{
    self, AddStorageInput, AddStorageOutput, EditStorageInput, EditStorageOutput,
    ExportConfigOutput, ImportConfigInput, ImportConfigOutput, ListStoragesOutput,
    RemoveStorageInput, RemoveStorageOutput, ValidateStorageInput, ValidateStorageOutput,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolCategory {
    Read,
    Write,
    Destructive,
    ExternalLink,
    Session,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
    pub category: McpToolCategory,
    pub risk: McpToolRisk,
    pub default_enabled: bool,
}

const SAFE_DEFAULT_TOOLS: &[&str] = &[
    "list_dir",
    "stat_path",
    "read_file",
    "search_paths",
    "list_versions",
    "read_file_version",
];

const ADMIN_TOOL_NAMES: &[&str] = &[
    "list_storages",
    "add_storage",
    "edit_storage",
    "remove_storage",
    "import_config",
    "export_config",
    "validate_storage",
];

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_dir",
            description: "List directories within the Infimount virtual filesystem.",
            input_schema: schemas::schema_list_dir(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "stat_path",
            description: "Return metadata for a filesystem path.",
            input_schema: schemas::schema_stat_path(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "read_file",
            description: "Read a file as UTF-8 text or base64 bytes with bounded size.",
            input_schema: schemas::schema_read_file(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "mkdir",
            description: "Create a directory and optional parent directories.",
            input_schema: schemas::schema_mkdir(),
            category: McpToolCategory::Write,
            risk: McpToolRisk::Medium,
            default_enabled: false,
        },
        ToolDefinition {
            name: "write_file",
            description: "Write UTF-8 text content to a file path.",
            input_schema: schemas::schema_write_file(),
            category: McpToolCategory::Write,
            risk: McpToolRisk::Medium,
            default_enabled: false,
        },
        ToolDefinition {
            name: "delete_path",
            description: "Delete a file or recursively delete a directory.",
            input_schema: schemas::schema_delete_path(),
            category: McpToolCategory::Destructive,
            risk: McpToolRisk::High,
            default_enabled: false,
        },
        ToolDefinition {
            name: "copy_path",
            description: "Copy a file or directory tree between filesystem paths.",
            input_schema: schemas::schema_copy_path(),
            category: McpToolCategory::Write,
            risk: McpToolRisk::Medium,
            default_enabled: false,
        },
        ToolDefinition {
            name: "move_path",
            description: "Move a file between filesystem paths.",
            input_schema: schemas::schema_move_path(),
            category: McpToolCategory::Destructive,
            risk: McpToolRisk::High,
            default_enabled: false,
        },
        ToolDefinition {
            name: "search_paths",
            description: "Recursively search for matching paths below a directory.",
            input_schema: schemas::schema_search_paths(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "generate_download_link",
            description: "Generate a presigned download link for a file path when supported.",
            input_schema: schemas::schema_generate_download_link(),
            category: McpToolCategory::ExternalLink,
            risk: McpToolRisk::Medium,
            default_enabled: false,
        },
        ToolDefinition {
            name: "session_create",
            description:
                "Create a scoped session with restricted access to specific storages and paths.",
            input_schema: schemas::schema_session_create(),
            category: McpToolCategory::Session,
            risk: McpToolRisk::Low,
            default_enabled: false,
        },
        ToolDefinition {
            name: "session_end",
            description: "Terminate an active session.",
            input_schema: schemas::schema_session_end(),
            category: McpToolCategory::Session,
            risk: McpToolRisk::Low,
            default_enabled: false,
        },
        ToolDefinition {
            name: "list_versions",
            description: "List all available versions of a file.",
            input_schema: schemas::schema_list_versions(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "read_file_version",
            description: "Read a specific version of a file.",
            input_schema: schemas::schema_read_file_version(),
            category: McpToolCategory::Read,
            risk: McpToolRisk::Low,
            default_enabled: true,
        },
        ToolDefinition {
            name: "delete_version",
            description: "Delete a specific version of a file.",
            input_schema: schemas::schema_delete_version(),
            category: McpToolCategory::Destructive,
            risk: McpToolRisk::High,
            default_enabled: false,
        },
    ]
}

pub fn rmcp_tools() -> Vec<Tool> {
    tool_definitions()
        .into_iter()
        .map(|definition| {
            Tool::new(
                definition.name,
                definition.description,
                Arc::new(schema_to_object(definition.input_schema)),
            )
            .with_annotations(tool_annotations(definition.name))
        })
        .collect()
}

pub fn all_tool_names() -> Vec<String> {
    let mut names = tool_definitions()
        .into_iter()
        .map(|definition| definition.name.to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

pub fn default_enabled_tool_names() -> Vec<String> {
    SAFE_DEFAULT_TOOLS
        .iter()
        .map(|name| name.to_string())
        .collect()
}

pub fn admin_tool_names() -> Vec<String> {
    ADMIN_TOOL_NAMES
        .iter()
        .map(|name| name.to_string())
        .collect()
}

fn filtered_tool_definitions(enabled_tools: &HashSet<String>) -> Vec<ToolDefinition> {
    tool_definitions()
        .into_iter()
        .filter(|definition| enabled_tools.contains(definition.name))
        .collect()
}

fn rmcp_tools_for(enabled_tools: &HashSet<String>) -> Vec<Tool> {
    filtered_tool_definitions(enabled_tools)
        .into_iter()
        .map(|definition| {
            Tool::new(
                definition.name,
                definition.description,
                Arc::new(schema_to_object(definition.input_schema)),
            )
            .with_annotations(tool_annotations(definition.name))
        })
        .collect()
}

fn normalize_enabled_tools(enabled_tools: Vec<String>) -> HashSet<String> {
    let available = all_tool_names().into_iter().collect::<HashSet<_>>();
    enabled_tools
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && available.contains(value))
        .collect()
}

#[derive(Debug)]
pub struct InfimountMcpServer {
    ctx: FsToolsContext,
    enabled_tools: HashSet<String>,
    telemetry: TelemetryState,
    audit: AuditStore,
    confirmations: ConfirmationManager,
}

impl InfimountMcpServer {
    pub fn new(ctx: FsToolsContext, enabled_tools: Vec<String>) -> Self {
        Self {
            ctx,
            enabled_tools: normalize_enabled_tools(enabled_tools),
            telemetry: TelemetryState::new(),
            audit: AuditStore::new(None),
            confirmations: ConfirmationManager::new(),
        }
    }

    pub fn with_confirmation_manager(
        ctx: FsToolsContext,
        enabled_tools: Vec<String>,
        confirmations: ConfirmationManager,
    ) -> Self {
        Self {
            ctx,
            enabled_tools: normalize_enabled_tools(enabled_tools),
            telemetry: TelemetryState::new(),
            audit: AuditStore::new(None),
            confirmations,
        }
    }

    fn is_tool_enabled(&self, tool_name: &str) -> bool {
        self.enabled_tools.contains(tool_name)
    }

    async fn dispatch_tool_json(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> Result<serde_json::Value, ErrorData> {
        if !self.is_tool_enabled(name) {
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        }

        let raw_input = serde_json::Value::Object(arguments.clone().unwrap_or_default());
        if let Some(arguments) = arguments.as_ref() {
            if let Some(response) = self.confirmation_gate(name, arguments).await? {
                return Ok(wrap_json(Ok(response)));
            }
        }

        let result = match name {
            "list_dir" => invoke_list_dir_json(&self.ctx, raw_input).await,
            "stat_path" => invoke_stat_path_json(&self.ctx, raw_input).await,
            "read_file" => invoke_read_file_json(&self.ctx, raw_input).await,
            "mkdir" => invoke_mkdir_json(&self.ctx, raw_input).await,
            "write_file" => invoke_write_file_json(&self.ctx, raw_input).await,
            "delete_path" => invoke_delete_path_json(&self.ctx, raw_input).await,
            "copy_path" => invoke_copy_path_json(&self.ctx, raw_input).await,
            "move_path" => invoke_move_path_json(&self.ctx, raw_input).await,
            "search_paths" => invoke_search_paths_json(&self.ctx, raw_input).await,
            "generate_download_link" => {
                invoke_generate_download_link_json(&self.ctx, raw_input).await
            }
            "session_create" => invoke_session_create_json(&self.ctx, raw_input).await,
            "session_end" => invoke_session_end_json(&self.ctx, raw_input).await,
            "list_versions" => invoke_list_versions_json(&self.ctx, raw_input).await,
            "read_file_version" => invoke_read_file_version_json(&self.ctx, raw_input).await,
            "delete_version" => invoke_delete_version_json(&self.ctx, raw_input).await,
            _ => {
                return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
            }
        };

        Ok(result)
    }

    async fn confirmation_gate(
        &self,
        tool_name: &str,
        arguments: &JsonObject,
    ) -> Result<Option<ConfirmationRequiredResponse>, ErrorData> {
        let Some(check) = confirmation_check(&self.ctx, tool_name, arguments)
            .await
            .map_err(mcp_to_rmcp_error)?
        else {
            return Ok(None);
        };

        let fingerprint = request_fingerprint(tool_name, arguments);
        if let Some(confirmation_id) = arguments
            .get("confirmation_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
        {
            self.confirmations
                .consume_approved(confirmation_id, &fingerprint)
                .await
                .map_err(mcp_to_rmcp_error)?;
            return Ok(None);
        }

        let pending = self
            .confirmations
            .require_confirmation(ConfirmationRequest {
                tool_name: tool_name.to_string(),
                operation: check.operation,
                risk_type: check.risk_type,
                storage_id: check.storage_id,
                storage_name: check.storage_name,
                path: check.path.clone(),
                summary: check.summary,
                request_fingerprint: fingerprint,
            })
            .await;

        let (eval_rule_id, eval_workspace_id) =
            crate::tools_fs::take_last_policy_eval().unwrap_or((None, None));
        self.audit_tool_call(AuditToolCall {
            tool_name,
            normalized_path: Some(&check.path),
            storage_name: Some(&pending.storage_name),
            decision: AuditDecision::RequiresConfirmation,
            matched_rule_id: eval_rule_id.as_deref(),
            workspace_id: eval_workspace_id.as_deref(),
            error_code: None,
            confirmation_id: Some(&pending.operation_id),
            duration_ms: 0,
        });

        Ok(Some(pending.into()))
    }
}

impl ServerHandler for InfimountMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools_with(ToolsCapability {
                    list_changed: Some(false),
                })
                .enable_resources_with(ResourcesCapability {
                    subscribe: Some(false),
                    list_changed: Some(false),
                })
                .enable_prompts_with(PromptsCapability {
                    list_changed: Some(false),
                })
                .build(),
        )
        .with_server_info(
            Implementation::new("infimount_mcp", env!("CARGO_PKG_VERSION"))
                .with_title("Infimount MCP Server")
                .with_description("Filesystem-style MCP server for Infimount storages."),
        )
        .with_instructions(
            "All filesystem tool paths must be absolute and use the Infimount virtual root. '/' lists mounted storages.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(rmcp_tools_for(
            &self.enabled_tools,
        )))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if !self.is_tool_enabled(name) {
            return None;
        }

        rmcp_tools_for(&self.enabled_tools)
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool_name = request.name.to_string();
        if !self.is_tool_enabled(&tool_name) {
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        }

        let normalized_path = normalized_path_log_ref(&tool_name, request.arguments.as_ref());
        let storage_ref = storage_log_ref(&tool_name, request.arguments.as_ref());
        let confirmation_id = confirmation_id_log_ref(request.arguments.as_ref());
        let started = Instant::now();

        self.telemetry.record_tool_call(&tool_name);

        let (result, request_attribution) = crate::tools_fs::with_last_policy_eval_scope(async {
            let result = self
                .dispatch_tool_json(request.name.as_ref(), request.arguments)
                .await;
            let attribution = crate::tools_fs::take_last_policy_eval();
            (result, attribution)
        })
        .await;

        let latency_ms = started.elapsed().as_millis() as f64;
        self.telemetry.record_latency(&tool_name, latency_ms);

        let (eval_rule_id, eval_workspace_id) = request_attribution.unwrap_or((None, None));

        match result {
            Err(e) => {
                let error_code = error_code_from_error_data(&e);
                let (error_rule_id, error_workspace_id) = e
                    .data
                    .as_ref()
                    .and_then(|data| data.get("details"))
                    .and_then(policy_attribution)
                    .unwrap_or_else(|| (eval_rule_id.clone(), eval_workspace_id.clone()));
                self.telemetry.record_error(error_code);
                self.audit_tool_call(AuditToolCall {
                    tool_name: &tool_name,
                    normalized_path: normalized_path.as_deref(),
                    storage_name: storage_ref.as_deref(),
                    decision: audit_decision_for_error(error_code),
                    matched_rule_id: error_rule_id.as_deref(),
                    workspace_id: error_workspace_id.as_deref(),
                    error_code: Some(error_code),
                    confirmation_id: confirmation_id.as_deref(),
                    duration_ms: latency_ms as u64,
                });
                info!(
                    tool = tool_name.as_str(),
                    path = normalized_path.as_deref().unwrap_or("-"),
                    storage = storage_ref.as_deref().unwrap_or("-"),
                    error_code,
                    latency_ms,
                    "mcp tool failed"
                );
                let error_result = json!({
                    "ok": false,
                    "error": {
                        "code": error_code,
                        "message": e.message,
                        "details": e.data.clone().unwrap_or(json!({}))
                    }
                });
                Ok(CallToolResult::structured_error(error_result))
            }
            Ok(result) => {
                let is_error = result
                    .get("ok")
                    .and_then(|value| value.as_bool())
                    .map(|ok| !ok)
                    .unwrap_or(true);

                if is_error {
                    let (result_rule_id, result_workspace_id) = result
                        .get("error")
                        .and_then(|error| error.get("details"))
                        .and_then(policy_attribution)
                        .unwrap_or_else(|| (eval_rule_id.clone(), eval_workspace_id.clone()));
                    let error_code = result
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(|code| code.as_str())
                        .unwrap_or("ERR_INTERNAL");
                    self.telemetry.record_error(error_code);
                    self.audit_tool_call(AuditToolCall {
                        tool_name: &tool_name,
                        normalized_path: normalized_path.as_deref(),
                        storage_name: storage_ref.as_deref(),
                        decision: audit_decision_for_error(error_code),
                        matched_rule_id: result_rule_id.as_deref(),
                        workspace_id: result_workspace_id.as_deref(),
                        error_code: Some(error_code),
                        confirmation_id: confirmation_id.as_deref(),
                        duration_ms: latency_ms as u64,
                    });
                    info!(
                        tool = tool_name.as_str(),
                        path = normalized_path.as_deref().unwrap_or("-"),
                        storage = storage_ref.as_deref().unwrap_or("-"),
                        error_code,
                        latency_ms,
                        "mcp tool failed"
                    );
                    Ok(CallToolResult::structured_error(result))
                } else {
                    if is_confirmation_required_response(&result) {
                        return Ok(CallToolResult::structured(result));
                    }
                    let decision = if confirmation_id.is_some() {
                        AuditDecision::Confirmed
                    } else {
                        AuditDecision::Allowed
                    };
                    self.audit_tool_call(AuditToolCall {
                        tool_name: &tool_name,
                        normalized_path: normalized_path.as_deref(),
                        storage_name: storage_ref.as_deref(),
                        decision,
                        matched_rule_id: eval_rule_id.as_deref(),
                        workspace_id: eval_workspace_id.as_deref(),
                        error_code: None,
                        confirmation_id: confirmation_id.as_deref(),
                        duration_ms: latency_ms as u64,
                    });
                    info!(
                        tool = tool_name.as_str(),
                        path = normalized_path.as_deref().unwrap_or("-"),
                        storage = storage_ref.as_deref().unwrap_or("-"),
                        latency_ms,
                        "mcp tool succeeded"
                    );
                    Ok(CallToolResult::structured(result))
                }
            }
        }
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        if !self.is_tool_enabled("list_dir") {
            return Err(ErrorData::method_not_found::<ListResourcesRequestMethod>());
        }

        let resources = resources::list_resources(&self.ctx).map_err(mcp_to_rmcp_error)?;
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        if !self.is_tool_enabled("stat_path") {
            return Err(ErrorData::method_not_found::<ReadResourceRequestMethod>());
        }

        let path = resources::resource_uri_to_mcp_path(&request.uri)
            .map_err(|message| ErrorData::invalid_params(message, None))?;
        let stat = tools_fs::stat_path(
            &self.ctx,
            StatPathInput {
                path: path.clone(),
                session_id: None,
            },
        )
        .await
        .map_err(mcp_to_rmcp_error)?;

        let required_tool = if stat.entry_type == tools_fs::EntryType::Dir {
            "list_dir"
        } else {
            "read_file"
        };
        if !self.is_tool_enabled(required_tool) {
            return Err(ErrorData::method_not_found::<ReadResourceRequestMethod>());
        }

        resources::read_resource(&self.ctx, &request.uri)
            .await
            .map_err(mcp_to_rmcp_error)
    }

    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::list_prompts()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<GetPromptResult, ErrorData> {
        prompts::get_prompt(request).map_err(|message| ErrorData::invalid_params(message, None))
    }
}

impl InfimountMcpServer {
    fn audit_tool_call(&self, call: AuditToolCall<'_>) {
        let mut event = AuditEvent::new(call.tool_name, operation_for_tool(call.tool_name));
        event.path = call.normalized_path.map(ToString::to_string);
        event.storage_name = call.storage_name.map(ToString::to_string);
        if let Some(storage_name) = call.storage_name {
            if let Ok(storages) = self.ctx.registry.load_all() {
                if let Some(storage) = storages
                    .into_iter()
                    .find(|storage| storage.name == storage_name)
                {
                    event.storage_id = Some(storage.id);
                    event.backend = Some(storage.backend);
                }
            }
        }
        event.decision = call.decision;
        event.error_code = call.error_code.map(ToString::to_string);
        event.matched_rule_id = call.matched_rule_id.map(ToString::to_string);
        event.workspace_id = call.workspace_id.map(ToString::to_string);
        event.confirmation_id = call.confirmation_id.map(ToString::to_string);
        event.duration_ms = Some(call.duration_ms);
        if let Err(error) = self.audit.append(event) {
            info!(
                tool = call.tool_name,
                error_code = ?error.code,
                "failed to write MCP audit event"
            );
        }
    }
}

struct AuditToolCall<'a> {
    tool_name: &'a str,
    normalized_path: Option<&'a str>,
    storage_name: Option<&'a str>,
    decision: AuditDecision,
    matched_rule_id: Option<&'a str>,
    workspace_id: Option<&'a str>,
    error_code: Option<&'a str>,
    confirmation_id: Option<&'a str>,
    duration_ms: u64,
}

pub async fn invoke_list_dir_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ListDirInput| async move {
            tools_fs::list_dir(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_stat_path_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: StatPathInput| async move {
            tools_fs::stat_path(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_read_file_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ReadFileInput| async move {
            tools_fs::read_file(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_mkdir_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: MkdirInput| async move {
            tools_fs::mkdir(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_write_file_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: WriteFileInput| async move {
            tools_fs::write_file(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_delete_path_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: DeletePathInput| async move {
            tools_fs::delete_path(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_copy_path_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: CopyPathInput| async move {
            tools_fs::copy_path(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_move_path_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: MovePathInput| async move {
            tools_fs::move_path(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_search_paths_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: SearchPathsInput| async move {
            tools_fs::search_paths(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_generate_download_link_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: GenerateDownloadLinkInput| async move {
            tools_fs::generate_download_link(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_session_create_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: SessionCreateInput| async move {
            let storages = ctx.registry.load_all()?;
            let session_read_only = input.read_only.unwrap_or(false);
            let prefixes = input.allowed_prefixes.clone().unwrap_or_default();

            for storage_name in &input.allowed_storages {
                let Some(storage) = storages.iter().find(|s| s.name == *storage_name) else {
                    return Err(err_with_details(
                        McpErrorCode::ERR_STORAGE_NOT_FOUND,
                        format!("Storage '{storage_name}' not found"),
                        json!({ "storage_name": storage_name }),
                    ));
                };

                if !storage.enabled {
                    return Err(err_with_details(
                        McpErrorCode::ERR_STORAGE_DISABLED,
                        format!("Storage '{storage_name}' is disabled"),
                        json!({ "storage_name": storage_name }),
                    ));
                }

                if !storage.mcp_exposed {
                    return Err(err_with_details(
                        McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
                        format!("Storage '{storage_name}' is not exposed to MCP"),
                        json!({ "storage_name": storage_name }),
                    ));
                }

                for prefix in &prefixes {
                    evaluate_storage_policy(
                        storage,
                        prefix,
                        McpOperation::Read,
                        false,
                        false,
                    )?;

                    if !session_read_only {
                        let write_result = evaluate_storage_policy(
                            storage,
                            prefix,
                            McpOperation::Write,
                            false,
                            false,
                        );
                        match write_result {
                            Ok(eval) if matches!(eval.decision, PolicyDecision::Allow | PolicyDecision::RequireConfirmation { .. }) => {}
                            _ => {
                                return Err(err_with_details(
                                    McpErrorCode::ERR_SESSION_FORBIDDEN,
                                    format!(
                                        "Cannot create writable session for '{prefix}': effective access is read-only"
                                    ),
                                    json!({ "storage_name": storage_name, "prefix": prefix }),
                                ));
                            }
                        }
                    }
                }
            }

            let session = ctx
                .sessions
                .create_session(
                    input.allowed_storages,
                    input.allowed_prefixes,
                    input.read_only,
                    input.ttl_seconds,
                )
                .await?;
            Ok(SessionCreateOutput {
                session_id: session.id,
            })
        })
        .await,
    )
}

pub async fn invoke_session_end_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: SessionEndInput| async move {
            let ended = ctx.sessions.end_session(&input.session_id).await?;
            Ok(SessionEndOutput {
                session_id: input.session_id,
                ended,
            })
        })
        .await,
    )
}

pub async fn invoke_list_versions_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ListVersionsInput| async move {
            let input_with_session = tools_fs::ListVersionsInput {
                path: input.path,
                limit: input.limit,
                cursor: input.cursor,
                session_id: input.session_id,
            };
            tools_fs::list_versions(ctx, input_with_session).await
        })
        .await,
    )
}

pub async fn invoke_read_file_version_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ReadFileVersionInput| async move {
            let input_with_session = tools_fs::ReadFileVersionInput {
                path: input.path,
                version: input.version,
                offset_bytes: input.offset_bytes,
                max_bytes: input.max_bytes,
                as_text: input.as_text,
                encoding: input.encoding,
                session_id: input.session_id,
            };
            tools_fs::read_file_version(ctx, input_with_session).await
        })
        .await,
    )
}

pub async fn invoke_delete_version_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: DeleteVersionInput| async move {
            let input_with_session = tools_fs::DeleteVersionInput {
                path: input.path,
                version: input.version,
                session_id: input.session_id,
                confirmation_id: input.confirmation_id,
            };
            tools_fs::delete_version(ctx, input_with_session).await
        })
        .await,
    )
}

async fn invoke_typed<Input, Output, F, Fut>(
    raw_input: serde_json::Value,
    handler: F,
) -> McpResult<Output>
where
    Input: DeserializeOwned,
    Output: Serialize,
    F: FnOnce(Input) -> Fut,
    Fut: std::future::Future<Output = McpResult<Output>>,
{
    let typed_input: Input = serde_json::from_value(raw_input).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "input does not match tool schema",
            json!({ "parse_error": e.to_string() }),
        )
    })?;

    handler(typed_input).await
}

pub async fn invoke_list_dir_typed(
    ctx: &FsToolsContext,
    input: ListDirInput,
) -> McpResult<ListDirOutput> {
    tools_fs::list_dir(ctx, input).await
}

pub async fn invoke_stat_path_typed(
    ctx: &FsToolsContext,
    input: StatPathInput,
) -> McpResult<StatPathOutput> {
    tools_fs::stat_path(ctx, input).await
}

pub async fn invoke_read_file_typed(
    ctx: &FsToolsContext,
    input: ReadFileInput,
) -> McpResult<ReadFileOutput> {
    tools_fs::read_file(ctx, input).await
}

pub async fn invoke_mkdir_typed(ctx: &FsToolsContext, input: MkdirInput) -> McpResult<MkdirOutput> {
    tools_fs::mkdir(ctx, input).await
}

pub async fn invoke_write_file_typed(
    ctx: &FsToolsContext,
    input: WriteFileInput,
) -> McpResult<WriteFileOutput> {
    tools_fs::write_file(ctx, input).await
}

pub async fn invoke_delete_path_typed(
    ctx: &FsToolsContext,
    input: DeletePathInput,
) -> McpResult<DeletePathOutput> {
    tools_fs::delete_path(ctx, input).await
}

pub async fn invoke_copy_path_typed(
    ctx: &FsToolsContext,
    input: CopyPathInput,
) -> McpResult<CopyPathOutput> {
    tools_fs::copy_path(ctx, input).await
}

pub async fn invoke_move_path_typed(
    ctx: &FsToolsContext,
    input: MovePathInput,
) -> McpResult<MovePathOutput> {
    tools_fs::move_path(ctx, input).await
}

pub async fn invoke_search_paths_typed(
    ctx: &FsToolsContext,
    input: SearchPathsInput,
) -> McpResult<SearchPathsOutput> {
    tools_fs::search_paths(ctx, input).await
}

pub async fn invoke_generate_download_link_typed(
    ctx: &FsToolsContext,
    input: GenerateDownloadLinkInput,
) -> McpResult<GenerateDownloadLinkOutput> {
    tools_fs::generate_download_link(ctx, input).await
}

pub async fn invoke_list_storages_typed(ctx: &FsToolsContext) -> McpResult<ListStoragesOutput> {
    tools_storage::list_storages(ctx).await
}

pub async fn invoke_add_storage_typed(
    ctx: &FsToolsContext,
    input: AddStorageInput,
) -> McpResult<AddStorageOutput> {
    tools_storage::add_storage(ctx, input).await
}

pub async fn invoke_edit_storage_typed(
    ctx: &FsToolsContext,
    input: EditStorageInput,
) -> McpResult<EditStorageOutput> {
    tools_storage::edit_storage(ctx, input).await
}

pub async fn invoke_remove_storage_typed(
    ctx: &FsToolsContext,
    input: RemoveStorageInput,
) -> McpResult<RemoveStorageOutput> {
    tools_storage::remove_storage(ctx, input).await
}

pub async fn invoke_import_config_typed(
    ctx: &FsToolsContext,
    input: ImportConfigInput,
) -> McpResult<ImportConfigOutput> {
    tools_storage::import_config(ctx, input).await
}

pub async fn invoke_export_config_typed(
    ctx: &FsToolsContext,
) -> McpResult<ExportConfigOutput> {
    tools_storage::export_config(ctx).await
}

pub async fn invoke_validate_storage_typed(
    ctx: &FsToolsContext,
    input: ValidateStorageInput,
) -> McpResult<ValidateStorageOutput> {
    tools_storage::validate_storage(ctx, input).await
}

fn schema_to_object(schema: serde_json::Value) -> JsonObject {
    match schema {
        serde_json::Value::Object(map) => map,
        _ => JsonObject::default(),
    }
}

fn tool_annotations(name: &str) -> ToolAnnotations {
    match name {
        "list_dir" | "stat_path" | "read_file" | "search_paths" | "list_versions"
        | "read_file_version" => ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "generate_download_link" => ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
        "session_create" | "session_end" => ToolAnnotations::new()
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "mkdir" | "write_file" | "copy_path" => ToolAnnotations::new()
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "move_path" | "delete_path" | "delete_version" => ToolAnnotations::new()
            .read_only(false)
            .destructive(true)
            .idempotent(false)
            .open_world(false),
        _ => ToolAnnotations::new()
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
    }
}

fn mcp_to_rmcp_error(error: crate::errors::McpError) -> ErrorData {
    let data = Some(json!({
        "code": error.code,
        "details": error.details
    }));

    match error.code {
        McpErrorCode::ERR_INTERNAL | McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT => {
            ErrorData::internal_error(error.message, data)
        }
        _ => ErrorData::invalid_params(error.message, data),
    }
}

fn storage_log_ref(name: &str, arguments: Option<&JsonObject>) -> Option<String> {
    let args = arguments?;

    match name {
        "list_dir"
        | "stat_path"
        | "read_file"
        | "write_file"
        | "mkdir"
        | "delete_path"
        | "search_paths"
        | "generate_download_link"
        | "list_versions"
        | "read_file_version"
        | "delete_version" => path_storage_name(args.get("path").and_then(|value| value.as_str())),
        "copy_path" | "move_path" => {
            let src = path_storage_name(args.get("src").and_then(|value| value.as_str()));
            let dst = path_storage_name(args.get("dst").and_then(|value| value.as_str()));
            match (src, dst) {
                (Some(src), Some(dst)) if src == dst => Some(src),
                (Some(src), Some(dst)) => Some(format!("{src}->{dst}")),
                (Some(src), None) => Some(src),
                (None, Some(dst)) => Some(dst),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

fn normalized_path_log_ref(name: &str, arguments: Option<&JsonObject>) -> Option<String> {
    let args = arguments?;

    match name {
        "list_dir"
        | "stat_path"
        | "read_file"
        | "write_file"
        | "mkdir"
        | "delete_path"
        | "search_paths"
        | "generate_download_link"
        | "list_versions"
        | "read_file_version"
        | "delete_version" => {
            normalize_logged_path(args.get("path").and_then(|value| value.as_str()))
        }
        "copy_path" | "move_path" => {
            let src = normalize_logged_path(args.get("src").and_then(|value| value.as_str()));
            let dst = normalize_logged_path(args.get("dst").and_then(|value| value.as_str()));
            match (src, dst) {
                (Some(src), Some(dst)) => Some(format!("{src} -> {dst}")),
                (Some(src), None) => Some(src),
                (None, Some(dst)) => Some(dst),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

fn path_storage_name(path: Option<&str>) -> Option<String> {
    let path = path?;
    let parsed = crate::path::parse_mcp_path(path).ok()?;
    parsed.storage_name
}

fn normalize_logged_path(path: Option<&str>) -> Option<String> {
    let path = path?;
    crate::path::parse_mcp_path(path)
        .ok()
        .map(|parsed| parsed.normalized)
}

#[derive(Debug)]
struct ConfirmationCheck {
    operation: McpOperation,
    risk_type: McpRiskType,
    storage_id: String,
    storage_name: String,
    path: String,
    summary: String,
}

async fn confirmation_check(
    ctx: &FsToolsContext,
    tool_name: &str,
    arguments: &JsonObject,
) -> McpResult<Option<ConfirmationCheck>> {
    match tool_name {
        "write_file" => {
            check_single_path_confirmation(
                ctx,
                tool_name,
                arguments,
                "path",
                McpOperation::Write,
                bool_arg(arguments, "overwrite", true),
                false,
            )
            .await
        }
        "mkdir" => {
            check_single_path_confirmation(
                ctx,
                tool_name,
                arguments,
                "path",
                McpOperation::Mkdir,
                false,
                false,
            )
            .await
        }
        "delete_path" => {
            check_single_path_confirmation(
                ctx,
                tool_name,
                arguments,
                "path",
                McpOperation::Delete,
                false,
                false,
            )
            .await
        }
        "generate_download_link" => {
            check_single_path_confirmation(
                ctx,
                tool_name,
                arguments,
                "path",
                McpOperation::PresignDownloadLink,
                false,
                false,
            )
            .await
        }
        "delete_version" => {
            check_single_path_confirmation(
                ctx,
                tool_name,
                arguments,
                "path",
                McpOperation::DeleteVersion,
                false,
                false,
            )
            .await
        }
        "copy_path" => {
            check_transfer_confirmation(
                ctx,
                tool_name,
                arguments,
                McpOperation::Copy,
                bool_arg(arguments, "overwrite", false),
            )
            .await
        }
        "move_path" => {
            check_transfer_confirmation(
                ctx,
                tool_name,
                arguments,
                McpOperation::Move,
                bool_arg(arguments, "overwrite", false),
            )
            .await
        }
        _ => Ok(None),
    }
}

async fn check_single_path_confirmation(
    ctx: &FsToolsContext,
    tool_name: &str,
    arguments: &JsonObject,
    path_key: &str,
    operation: McpOperation,
    overwrite: bool,
    cross_storage: bool,
) -> McpResult<Option<ConfirmationCheck>> {
    let Some(path) = arguments.get(path_key).and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let parsed = crate::path::parse_mcp_path(path)?;
    if parsed.is_root {
        return Ok(None);
    }
    let resolved = crate::path::resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let session_access = ctx
        .validate_session(
            session_id_arg(arguments),
            &resolved.storage.name,
            Some(&resolved.parsed.backend_path),
        )
        .await?;
    if session_access.read_only && operation.is_write_like() {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": session_id_arg(arguments) }),
        ));
    }

    let evaluation = evaluate_storage_policy(
        &resolved.storage,
        &resolved.parsed.backend_path,
        operation,
        overwrite,
        cross_storage,
    )?;
    crate::tools_fs::record_policy_eval(&evaluation);
    match evaluation.decision {
        PolicyDecision::Allow => Ok(None),
        PolicyDecision::RequireConfirmation { risk_type } => Ok(Some(ConfirmationCheck {
            operation,
            risk_type,
            storage_id: resolved.storage.id,
            storage_name: resolved.storage.name,
            path: parsed.normalized.clone(),
            summary: format!("{tool_name} on {}", parsed.normalized),
        })),
    }
}

async fn check_transfer_confirmation(
    ctx: &FsToolsContext,
    tool_name: &str,
    arguments: &JsonObject,
    operation: McpOperation,
    overwrite: bool,
) -> McpResult<Option<ConfirmationCheck>> {
    let Some(src) = arguments.get("src").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let Some(dst) = arguments.get("dst").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let src_parsed = crate::path::parse_mcp_path(src)?;
    let dst_parsed = crate::path::parse_mcp_path(dst)?;
    if src_parsed.is_root || dst_parsed.is_root {
        return Ok(None);
    }
    let src_resolved = crate::path::resolve_storage_path(&ctx.registry, &src_parsed.normalized)?;
    let dst_resolved = crate::path::resolve_storage_path(&ctx.registry, &dst_parsed.normalized)?;
    let src_session_access = ctx
        .validate_session(
            session_id_arg(arguments),
            &src_resolved.storage.name,
            Some(&src_resolved.parsed.backend_path),
        )
        .await?;
    let dst_session_access = ctx
        .validate_session(
            session_id_arg(arguments),
            &dst_resolved.storage.name,
            Some(&dst_resolved.parsed.backend_path),
        )
        .await?;
    let cross_storage = src_resolved.storage.id != dst_resolved.storage.id;
    let source_operation = if operation == McpOperation::Copy {
        McpOperation::Read
    } else {
        operation
    };

    if src_session_access.read_only && source_operation.is_write_like() {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": session_id_arg(arguments) }),
        ));
    }
    if dst_session_access.read_only && operation.is_write_like() {
        return Err(err_with_details(
            McpErrorCode::ERR_SESSION_FORBIDDEN,
            "session is read-only",
            json!({ "session_id": session_id_arg(arguments) }),
        ));
    }

    let src_eval = evaluate_storage_policy(
        &src_resolved.storage,
        &src_resolved.parsed.backend_path,
        source_operation,
        false,
        false,
    )?;
    crate::tools_fs::record_policy_eval(&src_eval);
    if let PolicyDecision::RequireConfirmation { risk_type } = src_eval.decision {
        return Ok(Some(ConfirmationCheck {
            operation,
            risk_type,
            storage_id: src_resolved.storage.id,
            storage_name: src_resolved.storage.name,
            path: src_parsed.normalized.clone(),
            summary: format!(
                "{tool_name} from {} to {}",
                src_parsed.normalized, dst_parsed.normalized
            ),
        }));
    }

    let destination_evaluation = evaluate_storage_policy(
        &dst_resolved.storage,
        &dst_resolved.parsed.backend_path,
        operation,
        overwrite,
        cross_storage,
    )?;
    crate::tools_fs::record_policy_eval(&destination_evaluation);
    match destination_evaluation.decision {
        PolicyDecision::Allow => Ok(None),
        PolicyDecision::RequireConfirmation { risk_type } => Ok(Some(ConfirmationCheck {
            operation,
            risk_type,
            storage_id: dst_resolved.storage.id,
            storage_name: dst_resolved.storage.name,
            path: dst_parsed.normalized.clone(),
            summary: format!(
                "{tool_name} from {} to {}",
                src_parsed.normalized, dst_parsed.normalized
            ),
        })),
    }
}

fn session_id_arg(arguments: &JsonObject) -> Option<&str> {
    arguments
        .get("session_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
}

fn bool_arg(arguments: &JsonObject, key: &str, default: bool) -> bool {
    arguments
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

fn confirmation_id_log_ref(arguments: Option<&JsonObject>) -> Option<String> {
    arguments?
        .get("confirmation_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
}

fn policy_attribution(value: &serde_json::Value) -> Option<(Option<String>, Option<String>)> {
    let rule_id = value
        .get("matched_rule_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let workspace_id = value
        .get("workspace_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    (rule_id.is_some() || workspace_id.is_some()).then_some((rule_id, workspace_id))
}

fn error_code_from_error_data(error: &ErrorData) -> &str {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(|code| code.as_str())
        .unwrap_or("ERR_INTERNAL")
}

fn audit_decision_for_error(error_code: &str) -> AuditDecision {
    match error_code {
        "ERR_MCP_POLICY_DENIED"
        | "ERR_STORAGE_DISABLED"
        | "ERR_STORAGE_NOT_EXPOSED"
        | "ERR_STORAGE_READ_ONLY"
        | "ERR_SESSION_FORBIDDEN"
        | "ERR_UNAUTHORIZED" => AuditDecision::Denied,
        _ => AuditDecision::Failed,
    }
}

fn is_confirmation_required_response(result: &serde_json::Value) -> bool {
    result
        .get("data")
        .and_then(|data| data.get("status"))
        .and_then(|status| status.as_str())
        == Some("requires_confirmation")
}

fn request_fingerprint(tool_name: &str, arguments: &JsonObject) -> String {
    let mut cloned = arguments.clone();
    cloned.remove("confirmation_id");
    let payload = serde_json::json!({
        "tool": tool_name,
        "arguments": cloned
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| tool_name.to_string())
}

fn operation_for_tool(tool_name: &str) -> McpOperation {
    match tool_name {
        "list_dir" => McpOperation::List,
        "stat_path" => McpOperation::Metadata,
        "read_file" => McpOperation::Read,
        "mkdir" => McpOperation::Mkdir,
        "write_file" => McpOperation::Write,
        "delete_path" => McpOperation::Delete,
        "copy_path" => McpOperation::Copy,
        "move_path" => McpOperation::Move,
        "search_paths" => McpOperation::Search,
        "generate_download_link" => McpOperation::PresignDownloadLink,
        "list_versions" => McpOperation::ListVersions,
        "read_file_version" => McpOperation::ReadFileVersion,
        "delete_version" => McpOperation::DeleteVersion,
        "session_create" | "session_end" => McpOperation::Metadata,
        _ => McpOperation::Metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::policy::{
        McpAccessMode, McpConfirmationRules, McpPathRule, McpRuleSource, McpStoragePolicy,
    };
    use crate::registry::{StorageRecord, StorageRegistry};
    use crate::session::SessionManager;

    fn test_context(temp_dir: &TempDir) -> FsToolsContext {
        FsToolsContext {
            registry: StorageRegistry::new(Some(temp_dir.path().join("storages.json"))),
            sessions: SessionManager::new(),
            allow_insecure: true,
            auth_token: None,
        }
    }

    #[test]
    fn default_enabled_tools_are_safe_read_only_set() {
        assert_eq!(
            default_enabled_tool_names(),
            SAFE_DEFAULT_TOOLS
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_enabled_tools_do_not_include_admin_tools() {
        let default_names = default_enabled_tool_names();
        for admin in ADMIN_TOOL_NAMES {
            assert!(
                !default_names.contains(&admin.to_string()),
                "admin tool {} should not be in default set",
                admin
            );
        }
    }

    #[test]
    fn admin_tools_not_in_tool_definitions() {
        let names: Vec<&str> = tool_definitions().iter().map(|t| t.name).collect();
        for admin in ADMIN_TOOL_NAMES {
            assert!(
                !names.contains(admin),
                "admin tool {} should not be in tool_definitions",
                admin
            );
        }
    }

    #[test]
    fn tool_definitions_have_metadata() {
        for def in tool_definitions() {
            assert!(!def.name.is_empty());
            assert!(!def.description.is_empty());
            // Every tool must have a category, risk, and default_enabled
            let _ = def.category;
            let _ = def.risk;
            let _ = def.default_enabled;
        }
    }

    #[test]
    fn all_safe_default_tools_have_default_enabled_true() {
        let defs = tool_definitions();
        for name in SAFE_DEFAULT_TOOLS {
            let def = defs.iter().find(|d| d.name == *name).unwrap();
            assert!(def.default_enabled, "{} should be default enabled", name);
        }
    }

    #[test]
    fn write_destructive_tools_have_default_enabled_false() {
        let disabled = [
            "mkdir",
            "write_file",
            "delete_path",
            "copy_path",
            "move_path",
            "delete_version",
            "generate_download_link",
            "session_create",
            "session_end",
        ];
        let defs = tool_definitions();
        for name in disabled {
            let def = defs.iter().find(|d| d.name == name).unwrap();
            assert!(
                !def.default_enabled,
                "{} should NOT be default enabled",
                name
            );
        }
    }

    #[test]
    fn tool_annotations_match_operation_semantics() {
        let read = tool_annotations("list_dir");
        assert_eq!(read.read_only_hint, Some(true));
        assert_eq!(read.destructive_hint, Some(false));
        assert_eq!(read.idempotent_hint, Some(true));
        assert_eq!(read.open_world_hint, Some(false));

        let link = tool_annotations("generate_download_link");
        assert_eq!(link.read_only_hint, Some(true));
        assert_eq!(link.destructive_hint, Some(false));
        assert_eq!(link.idempotent_hint, Some(true));
        assert_eq!(link.open_world_hint, Some(true));

        for name in ["mkdir", "write_file", "copy_path"] {
            let write = tool_annotations(name);
            assert_eq!(write.read_only_hint, Some(false));
            assert_eq!(write.destructive_hint, Some(false));
        }
        for name in ["move_path", "delete_path", "delete_version"] {
            let destructive = tool_annotations(name);
            assert_eq!(destructive.read_only_hint, Some(false));
            assert_eq!(destructive.destructive_hint, Some(true));
        }
        for name in ["session_create", "session_end"] {
            let session = tool_annotations(name);
            assert_eq!(session.read_only_hint, Some(false));
            assert_eq!(session.destructive_hint, Some(false));
        }
    }

    #[test]
    fn normalize_enabled_tools_filters_invalid_entries() {
        let normalized = normalize_enabled_tools(vec![
            "list_dir".to_string(),
            "  list_dir ".to_string(),
            "unknown_tool".to_string(),
            "".to_string(),
        ]);

        assert!(normalized.contains("list_dir"));
        assert_eq!(normalized.len(), 1);
    }

    #[test]
    fn filtered_tool_definitions_hides_disabled_tools() {
        let enabled = normalize_enabled_tools(vec!["list_dir".to_string()]);
        let definitions = filtered_tool_definitions(&enabled);
        let names = definitions
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["list_dir"]);
    }

    #[test]
    fn get_tool_returns_none_for_disabled_tools() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = InfimountMcpServer::new(test_context(&temp_dir), vec!["list_dir".into()]);

        assert!(server.get_tool("list_dir").is_some());
        assert!(server.get_tool("write_file").is_none());
    }

    #[tokio::test]
    async fn dispatch_rejects_admin_tool_as_method_not_found() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = InfimountMcpServer::new(test_context(&temp_dir), default_enabled_tool_names());

        for admin in ADMIN_TOOL_NAMES {
            let result = server
                .dispatch_tool_json(admin, Some(JsonObject::new()))
                .await;
            let error = result.expect_err("admin tool should return method-not-found");
            assert_eq!(
                error.code,
                ErrorData::method_not_found::<CallToolRequestMethod>().code,
                "admin tool {admin} should return method-not-found"
            );
        }
    }

    #[tokio::test]
    async fn dispatch_rejects_disabled_data_plane_tool() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = InfimountMcpServer::new(test_context(&temp_dir), vec!["list_dir".into()]);

        let result = server
            .dispatch_tool_json("write_file", Some(JsonObject::new()))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn session_creation_cannot_elevate_or_escape_workspace_grant() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let ctx = test_context(&temp_dir);
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": temp_dir.path() }),
        );
        storage.enabled = true;
        storage.mcp_exposed = true;
        storage.mcp_policy = McpStoragePolicy {
            default_access: McpAccessMode::None,
            rules: vec![McpPathRule {
                id: "workspace:w-1".to_string(),
                prefix: "workspace".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-1".to_string(),
                },
                confirmation_rules: None,
            }],
            ..McpStoragePolicy::default()
        };
        ctx.registry
            .save_all_atomic(&[storage])
            .expect("save storage");

        let writable = invoke_session_create_json(
            &ctx,
            json!({
                "allowed_storages": ["Local"],
                "allowed_prefixes": ["workspace"],
                "read_only": false
            }),
        )
        .await;
        assert_eq!(
            writable
                .pointer("/error/code")
                .and_then(|value| value.as_str()),
            Some("ERR_SESSION_FORBIDDEN")
        );

        let outside = invoke_session_create_json(
            &ctx,
            json!({
                "allowed_storages": ["Local"],
                "allowed_prefixes": ["outside"],
                "read_only": true
            }),
        )
        .await;
        assert_eq!(
            outside
                .pointer("/error/code")
                .and_then(|value| value.as_str()),
            Some("ERR_MCP_POLICY_DENIED")
        );

        let read_only = invoke_session_create_json(
            &ctx,
            json!({
                "allowed_storages": ["Local"],
                "allowed_prefixes": ["workspace"],
                "read_only": true
            }),
        )
        .await;
        assert_eq!(
            read_only.get("ok").and_then(|value| value.as_bool()),
            Some(true)
        );

        let mut saved = ctx.registry.load_all().expect("load storage");
        saved[0].mcp_exposed = false;
        ctx.registry.save_all_atomic(&saved).expect("hide storage");
        let hidden = invoke_session_create_json(
            &ctx,
            json!({ "allowed_storages": ["Local"], "read_only": true }),
        )
        .await;
        assert_eq!(
            hidden
                .pointer("/error/code")
                .and_then(|value| value.as_str()),
            Some("ERR_STORAGE_NOT_EXPOSED")
        );

        saved[0].mcp_exposed = true;
        saved[0].enabled = false;
        ctx.registry
            .save_all_atomic(&saved)
            .expect("disable storage");
        let disabled = invoke_session_create_json(
            &ctx,
            json!({ "allowed_storages": ["Local"], "read_only": true }),
        )
        .await;
        assert_eq!(
            disabled
                .pointer("/error/code")
                .and_then(|value| value.as_str()),
            Some("ERR_STORAGE_DISABLED")
        );
    }

    #[tokio::test]
    async fn mcp_scenario_allows_reads_denies_prefix_escape_and_blocks_read_only_session_write() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let root = temp_dir.path().join("local");
        std::fs::create_dir_all(root.join("public/private")).expect("create local root");
        std::fs::write(root.join("public/readme.txt"), "hello").expect("write readme");
        std::fs::write(root.join("public/private/secret.txt"), "secret").expect("write secret");

        let ctx = test_context(&temp_dir);
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": root.clone() }),
        );
        storage.mcp_exposed = true;
        storage.mcp_policy = McpStoragePolicy {
            default_access: McpAccessMode::ReadWrite,
            version: 2,
            rules: vec![crate::policy::McpPathRule {
                id: "public".to_string(),
                prefix: "public".to_string(),
                access: McpAccessMode::ReadWrite,
                source: crate::policy::McpRuleSource::Manual,
                confirmation_rules: None,
            }],
            denied_paths: vec!["public/private".to_string()],
            confirmation_rules: McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                require_for_delete: false,
                require_for_version_delete: false,
                require_for_presign: false,
                require_for_cross_storage_copy: false,
            },
            ..Default::default()
        };
        ctx.registry
            .save_all_atomic(&[storage])
            .expect("save registry");
        let sessions = ctx.sessions.clone();
        let all_enabled = all_tool_names();
        let server = InfimountMcpServer::new(ctx, all_enabled);

        let mut read_args = JsonObject::new();
        read_args.insert("path".to_string(), json!("/Local/public/readme.txt"));
        let read_response = server
            .dispatch_tool_json("read_file", Some(read_args))
            .await
            .expect("read response");
        assert_eq!(read_response["ok"], true);
        assert_eq!(read_response["data"]["content"], "hello");

        let mut denied_args = JsonObject::new();
        denied_args.insert(
            "path".to_string(),
            json!("/Local/public/private/../private/secret.txt"),
        );
        let denied_response = server
            .dispatch_tool_json("read_file", Some(denied_args))
            .await
            .expect("denied response");
        assert_eq!(denied_response["ok"], false);
        assert_eq!(denied_response["error"]["code"], "ERR_MCP_POLICY_DENIED");

        let session = sessions
            .create_session(vec!["Local".to_string()], None, Some(true), Some(60))
            .await
            .expect("create session");
        let mut write_args = JsonObject::new();
        write_args.insert("path".to_string(), json!("/Local/public/new.txt"));
        write_args.insert("content".to_string(), json!("new"));
        write_args.insert("session_id".to_string(), json!(session.id));
        let write_error = server
            .dispatch_tool_json("write_file", Some(write_args))
            .await
            .expect_err("write should be rejected before confirmation");
        assert_eq!(
            error_code_from_error_data(&write_error),
            "ERR_SESSION_FORBIDDEN"
        );
        assert!(!root.join("public/new.txt").exists());
    }

    #[tokio::test]
    async fn read_only_session_blocks_write_before_confirmation_prompt() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let root = temp_dir.path().join("local");
        std::fs::create_dir_all(&root).expect("create local root");

        let ctx = test_context(&temp_dir);
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": root.clone() }),
        );
        storage.mcp_exposed = true;
        storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
        ctx.registry
            .save_all_atomic(&[storage])
            .expect("save registry");
        let sessions = ctx.sessions.clone();
        let all_enabled = all_tool_names();
        let server = InfimountMcpServer::new(ctx, all_enabled);

        let session = sessions
            .create_session(vec!["Local".to_string()], None, Some(true), Some(60))
            .await
            .expect("create session");
        let mut args = JsonObject::new();
        args.insert("path".to_string(), json!("/Local/new.txt"));
        args.insert("content".to_string(), json!("new"));
        args.insert("session_id".to_string(), json!(session.id));

        let error = server
            .dispatch_tool_json("write_file", Some(args))
            .await
            .expect_err("write should be rejected before confirmation");

        assert_eq!(error_code_from_error_data(&error), "ERR_SESSION_FORBIDDEN");
        assert!(!root.join("new.txt").exists());
    }

    #[tokio::test]
    async fn copy_from_read_only_source_to_writable_destination_is_allowed() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let src_root = temp_dir.path().join("source");
        let dst_root = temp_dir.path().join("destination");
        std::fs::create_dir_all(&src_root).expect("create source root");
        std::fs::create_dir_all(&dst_root).expect("create destination root");
        std::fs::write(src_root.join("report.txt"), "hello").expect("write source file");

        let ctx = test_context(&temp_dir);
        let mut source = StorageRecord::new(
            "Source".to_string(),
            "local".to_string(),
            json!({ "root": src_root.clone() }),
        );
        source.mcp_exposed = true;
        source.mcp_policy.default_access = McpAccessMode::ReadWrite;
        source.read_only = true;
        source.mcp_policy = McpStoragePolicy {
            default_access: McpAccessMode::ReadOnly,
            ..Default::default()
        };
        let mut destination = StorageRecord::new(
            "Destination".to_string(),
            "local".to_string(),
            json!({ "root": dst_root.clone() }),
        );
        destination.mcp_exposed = true;
        destination.mcp_policy = McpStoragePolicy {
            confirmation_rules: McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                require_for_delete: false,
                require_for_version_delete: false,
                require_for_presign: false,
                require_for_cross_storage_copy: false,
            },
            default_access: McpAccessMode::ReadWrite,
            ..Default::default()
        };
        ctx.registry
            .save_all_atomic(&[source, destination])
            .expect("save registry");
        let all_enabled = all_tool_names();
        let server = InfimountMcpServer::new(ctx, all_enabled);

        let mut args = JsonObject::new();
        args.insert("src".to_string(), json!("/Source/report.txt"));
        args.insert("dst".to_string(), json!("/Destination/report.txt"));

        let response = server
            .dispatch_tool_json("copy_path", Some(args))
            .await
            .expect("copy response");

        assert_eq!(response["ok"], true);
        assert_eq!(response["data"]["copied"], true);
        assert_eq!(
            std::fs::read_to_string(dst_root.join("report.txt")).expect("read copied file"),
            "hello"
        );
    }

    #[tokio::test]
    async fn dispatch_requires_confirmation_then_approved_operation_executes_once() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let root = temp_dir.path().join("local");
        std::fs::create_dir_all(&root).expect("create local root");
        std::fs::write(root.join("file.txt"), "secret").expect("write test file");

        let ctx = test_context(&temp_dir);
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": root.clone() }),
        );
        storage.mcp_exposed = true;
        storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
        ctx.registry
            .save_all_atomic(&[storage])
            .expect("save registry");

        let confirmations = ConfirmationManager::new();
        let all_enabled = all_tool_names();
        let server =
            InfimountMcpServer::with_confirmation_manager(ctx, all_enabled, confirmations.clone());

        let mut args = JsonObject::new();
        args.insert("path".to_string(), json!("/Local/file.txt"));
        args.insert("recursive".to_string(), json!(false));

        let response = server
            .dispatch_tool_json("delete_path", Some(args.clone()))
            .await
            .expect("confirmation response");
        assert_eq!(response["ok"], true);
        assert_eq!(response["data"]["status"], "requires_confirmation");
        assert!(root.join("file.txt").exists());

        let operation_id = response["data"]["operation_id"]
            .as_str()
            .expect("operation id")
            .to_string();
        confirmations
            .approve(&operation_id)
            .await
            .expect("approve operation");

        args.insert("confirmation_id".to_string(), json!(operation_id));
        let delete_response = server
            .dispatch_tool_json("delete_path", Some(args.clone()))
            .await
            .expect("delete response");
        assert_eq!(delete_response["ok"], true);
        assert_eq!(delete_response["data"]["deleted"], true);
        assert!(!root.join("file.txt").exists());

        let replay = server.dispatch_tool_json("delete_path", Some(args)).await;
        assert!(replay.is_err());
    }

    #[tokio::test]
    async fn approved_confirmation_cannot_be_used_for_modified_request() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let root = temp_dir.path().join("local");
        std::fs::create_dir_all(&root).expect("create local root");
        std::fs::write(root.join("a.txt"), "a").expect("write file a");
        std::fs::write(root.join("b.txt"), "b").expect("write file b");

        let ctx = test_context(&temp_dir);
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": root.clone() }),
        );
        storage.mcp_exposed = true;
        storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
        ctx.registry
            .save_all_atomic(&[storage])
            .expect("save registry");

        let confirmations = ConfirmationManager::new();
        let all_enabled = all_tool_names();
        let server =
            InfimountMcpServer::with_confirmation_manager(ctx, all_enabled, confirmations.clone());

        let mut args = JsonObject::new();
        args.insert("path".to_string(), json!("/Local/a.txt"));
        args.insert("recursive".to_string(), json!(false));

        let response = server
            .dispatch_tool_json("delete_path", Some(args))
            .await
            .expect("confirmation response");
        let operation_id = response["data"]["operation_id"]
            .as_str()
            .expect("operation id")
            .to_string();
        confirmations
            .approve(&operation_id)
            .await
            .expect("approve operation");

        let mut modified_args = JsonObject::new();
        modified_args.insert("path".to_string(), json!("/Local/b.txt"));
        modified_args.insert("recursive".to_string(), json!(false));
        modified_args.insert("confirmation_id".to_string(), json!(operation_id));

        let result = server
            .dispatch_tool_json("delete_path", Some(modified_args))
            .await;

        assert!(result.is_err());
        assert!(root.join("a.txt").exists());
        assert!(root.join("b.txt").exists());
    }

    #[test]
    fn audit_helpers_classify_confirmation_and_policy_events() {
        assert_eq!(
            audit_decision_for_error("ERR_MCP_POLICY_DENIED"),
            AuditDecision::Denied
        );
        assert_eq!(
            audit_decision_for_error("ERR_STORAGE_NOT_EXPOSED"),
            AuditDecision::Denied
        );
        assert_eq!(
            audit_decision_for_error("ERR_CONFIRMATION_REQUIRED"),
            AuditDecision::Failed
        );

        let response = json!({
            "ok": true,
            "data": {
                "status": "requires_confirmation",
                "operation_id": "op-1"
            }
        });
        assert!(is_confirmation_required_response(&response));

        let response = json!({
            "ok": true,
            "data": {
                "deleted": true
            }
        });
        assert!(!is_confirmation_required_response(&response));
    }
}
