use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResult, ErrorData,
    GetPromptRequestParams, GetPromptResult, Implementation, JsonObject, ListPromptsResult,
    ListResourcesResult, ListToolsResult, PromptsCapability, ReadResourceRequestParams,
    ReadResourceResult, ResourcesCapability, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    ToolsCapability,
};
use rmcp::ServerHandler;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use tracing::info;

use crate::errors::{err_with_details, wrap_json, McpErrorCode, McpResult};
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
use crate::tools_storage::{
    self, AddStorageInput, AddStorageOutput, EditStorageInput, EditStorageOutput,
    ExportConfigInput, ExportConfigOutput, ImportConfigInput, ImportConfigOutput,
    ListStoragesInput, ListStoragesOutput, RemoveStorageInput, RemoveStorageOutput,
    ValidateStorageInput, ValidateStorageOutput,
};

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_dir",
            description: "List directories within the Infimount virtual filesystem.",
            input_schema: schemas::schema_list_dir(),
        },
        ToolDefinition {
            name: "stat_path",
            description: "Return metadata for a filesystem path.",
            input_schema: schemas::schema_stat_path(),
        },
        ToolDefinition {
            name: "read_file",
            description: "Read a file as UTF-8 text or base64 bytes with bounded size.",
            input_schema: schemas::schema_read_file(),
        },
        ToolDefinition {
            name: "mkdir",
            description: "Create a directory and optional parent directories.",
            input_schema: schemas::schema_mkdir(),
        },
        ToolDefinition {
            name: "write_file",
            description: "Write UTF-8 text content to a file path.",
            input_schema: schemas::schema_write_file(),
        },
        ToolDefinition {
            name: "delete_path",
            description: "Delete a file or recursively delete a directory.",
            input_schema: schemas::schema_delete_path(),
        },
        ToolDefinition {
            name: "copy_path",
            description: "Copy a file or directory tree between filesystem paths.",
            input_schema: schemas::schema_copy_path(),
        },
        ToolDefinition {
            name: "move_path",
            description: "Move a file between filesystem paths.",
            input_schema: schemas::schema_move_path(),
        },
        ToolDefinition {
            name: "search_paths",
            description: "Recursively search for matching paths below a directory.",
            input_schema: schemas::schema_search_paths(),
        },
        ToolDefinition {
            name: "generate_download_link",
            description: "Generate a presigned download link for a file path when supported.",
            input_schema: schemas::schema_generate_download_link(),
        },
        ToolDefinition {
            name: "list_storages",
            description: "List all configured storages with secrets masked.",
            input_schema: schemas::schema_list_storages(),
        },
        ToolDefinition {
            name: "add_storage",
            description: "Add a storage definition to the Infimount registry.",
            input_schema: schemas::schema_add_storage(),
        },
        ToolDefinition {
            name: "edit_storage",
            description: "Edit an existing storage definition by name.",
            input_schema: schemas::schema_edit_storage(),
        },
        ToolDefinition {
            name: "remove_storage",
            description: "Remove a storage definition by name.",
            input_schema: schemas::schema_remove_storage(),
        },
        ToolDefinition {
            name: "import_config",
            description: "Import storage registry JSON into the Infimount registry.",
            input_schema: schemas::schema_import_config(),
        },
        ToolDefinition {
            name: "export_config",
            description: "Export the storage registry as JSON with optional secret masking.",
            input_schema: schemas::schema_export_config(),
        },
        ToolDefinition {
            name: "validate_storage",
            description: "Validate a storage configuration and return backend capabilities.",
            input_schema: schemas::schema_validate_storage(),
        },
        ToolDefinition {
            name: "session_create",
            description:
                "Create a scoped session with restricted access to specific storages and paths.",
            input_schema: schemas::schema_session_create(),
        },
        ToolDefinition {
            name: "session_end",
            description: "Terminate an active session.",
            input_schema: schemas::schema_session_end(),
        },
        ToolDefinition {
            name: "list_versions",
            description: "List all available versions of a file.",
            input_schema: schemas::schema_list_versions(),
        },
        ToolDefinition {
            name: "read_file_version",
            description: "Read a specific version of a file.",
            input_schema: schemas::schema_read_file_version(),
        },
        ToolDefinition {
            name: "delete_version",
            description: "Delete a specific version of a file.",
            input_schema: schemas::schema_delete_version(),
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
    all_tool_names()
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
}

impl InfimountMcpServer {
    pub fn new(ctx: FsToolsContext, enabled_tools: Vec<String>) -> Self {
        Self {
            ctx,
            enabled_tools: normalize_enabled_tools(enabled_tools),
            telemetry: TelemetryState::new(),
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
            "list_storages" => invoke_list_storages_json(&self.ctx, raw_input).await,
            "add_storage" => invoke_add_storage_json(&self.ctx, raw_input).await,
            "edit_storage" => invoke_edit_storage_json(&self.ctx, raw_input).await,
            "remove_storage" => invoke_remove_storage_json(&self.ctx, raw_input).await,
            "import_config" => invoke_import_config_json(&self.ctx, raw_input).await,
            "export_config" => invoke_export_config_json(&self.ctx, raw_input).await,
            "validate_storage" => invoke_validate_storage_json(&self.ctx, raw_input).await,
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
        let started = Instant::now();

        self.telemetry.record_tool_call(&tool_name);

        let result = self
            .dispatch_tool_json(request.name.as_ref(), request.arguments)
            .await;

        let latency_ms = started.elapsed().as_millis() as f64;
        self.telemetry.record_latency(&tool_name, latency_ms);

        match result {
            Err(e) => {
                let error_code = "ERR_INTERNAL";
                self.telemetry.record_error(error_code);
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
                    let error_code = result
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(|code| code.as_str())
                        .unwrap_or("ERR_INTERNAL");
                    self.telemetry.record_error(error_code);
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
        let resources = resources::list_resources(&self.ctx).map_err(mcp_to_rmcp_error)?;
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
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

pub async fn invoke_list_storages_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |_: ListStoragesInput| async move {
            tools_storage::list_storages(ctx).await
        })
        .await,
    )
}

pub async fn invoke_add_storage_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: AddStorageInput| async move {
            tools_storage::add_storage(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_edit_storage_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: EditStorageInput| async move {
            tools_storage::edit_storage(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_remove_storage_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: RemoveStorageInput| async move {
            tools_storage::remove_storage(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_import_config_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ImportConfigInput| async move {
            tools_storage::import_config(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_export_config_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ExportConfigInput| async move {
            tools_storage::export_config(ctx, input).await
        })
        .await,
    )
}

pub async fn invoke_validate_storage_json(
    ctx: &FsToolsContext,
    raw_input: serde_json::Value,
) -> serde_json::Value {
    wrap_json(
        invoke_typed(raw_input, |input: ValidateStorageInput| async move {
            tools_storage::validate_storage(ctx, input).await
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
    input: ExportConfigInput,
) -> McpResult<ExportConfigOutput> {
    tools_storage::export_config(ctx, input).await
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
        "list_dir"
        | "stat_path"
        | "read_file"
        | "search_paths"
        | "generate_download_link"
        | "list_storages"
        | "export_config"
        | "validate_storage" => ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        _ => ToolAnnotations::new()
            .read_only(false)
            .destructive(true)
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
        | "generate_download_link" => {
            path_storage_name(args.get("path").and_then(|value| value.as_str()))
        }
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
        "list_storages" | "import_config" | "export_config" => None,
        "add_storage" | "remove_storage" | "validate_storage" => args
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        "edit_storage" => args
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
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
        | "generate_download_link" => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::registry::StorageRegistry;
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
    fn default_enabled_tools_match_all_tool_names() {
        let default_names = default_enabled_tool_names();
        let all_names = all_tool_names();
        assert_eq!(default_names, all_names);
        assert!(!default_names.is_empty());
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
        assert!(server.get_tool("export_config").is_none());
    }

    #[tokio::test]
    async fn dispatch_rejects_disabled_tool_without_running_handler() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = InfimountMcpServer::new(test_context(&temp_dir), vec!["list_dir".into()]);

        let result = server
            .dispatch_tool_json("export_config", Some(JsonObject::new()))
            .await;

        assert!(result.is_err());
    }
}
