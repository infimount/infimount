//! The hosted MCP surface has no ambient session or desktop authority.
//! Identity is copied only from the authenticated HTTP request extension.
use std::sync::Arc;

use axum::http::request::Parts;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResult, Content, ErrorData,
    Implementation, InitializeRequestParams, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations, ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::auth::Principal;
use crate::error::{HostedError, Result};
use crate::storage::StoragePlane;
use crate::transfers::TransferManager;
use crate::transport::RequestLifetime;

pub const READ_SCOPE: &str = "infimount:read";
pub const CREATE_SCOPE: &str = "infimount:create";
pub const DOWNLOAD_SCOPE: &str = "infimount:download";

#[derive(Clone)]
pub struct HostedMcpServer {
    storage: Arc<StoragePlane>,
    transfers: Arc<TransferManager>,
}

impl HostedMcpServer {
    pub fn new(storage: Arc<StoragePlane>, transfers: Arc<TransferManager>) -> Self {
        Self { storage, transfers }
    }

    async fn dispatch(
        &self,
        principal: &Principal,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<Value> {
        principal.ensure_live()?;
        if !allowed(principal, name) {
            return Err(HostedError::Forbidden);
        }
        let input = Value::Object(arguments.unwrap_or_default());
        match name {
            "list_dir" => {
                let input: ListInput = decode(input)?;
                self.storage
                    .list(principal, &input.path, input.limit, input.cursor)
                    .await
            }
            "stat_path" => {
                let input: PathInput = decode(input)?;
                self.storage.stat(principal, &input.path).await
            }
            "read_file" => {
                let input: ReadInput = decode(input)?;
                self.storage
                    .read(
                        principal,
                        &input.path,
                        input.offset,
                        input
                            .max_bytes
                            .unwrap_or(self.storage.config().limits.inline_bytes as u64),
                    )
                    .await
            }
            "write_file" => {
                let input: WriteInput = decode(input)?;
                self.transfers
                    .write_file(
                        principal,
                        &input.path,
                        &input.content,
                        &input.encoding,
                        input.content_type.as_deref(),
                    )
                    .await
            }
            "create_download" => {
                let input: DownloadInput = decode(input)?;
                self.transfers
                    .create_download(principal, &input.path, input.ttl_seconds)
                    .await
            }
            "create_upload" => {
                let input: UploadInput = decode(input)?;
                self.transfers
                    .create_upload(
                        principal,
                        &input.path,
                        input.size,
                        &input.sha256,
                        input.content_type.as_deref(),
                        input.ttl_seconds,
                    )
                    .await
            }
            "complete_upload" => {
                let input: TransferInput = decode(input)?;
                self.transfers.complete_upload(principal, &input.id).await
            }
            "abort_upload" => {
                let input: TransferInput = decode(input)?;
                self.transfers.abort_upload(principal, &input.id).await
            }
            _ => Err(HostedError::Unsupported),
        }
    }

    fn tools(&self) -> Vec<Tool> {
        let path = json!({"type":"string","minLength":1,"maxLength":4096});
        let ttl = json!({
            "type":"integer","minimum":1,
            "maximum":self.storage.config().limits.grant_ttl_seconds
        });
        let id = json!({"type":"string","minLength":1,"maxLength":128});
        let content_type = json!({"type":"string","minLength":1,"maxLength":128});
        let inline = self.storage.config().limits.inline_bytes;
        vec![
            tool(
                "list_dir",
                "List only authorized paths. Continuation cursors are bound to the authenticated principal and policy.",
                json!({"path":path,"limit":{"type":"integer","minimum":1,"maximum":200,"default":200},"cursor":{"type":"string","maxLength":4096}}),
                &["path"],
                true,
            ),
            tool(
                "stat_path",
                "Inspect one authorized storage path.",
                json!({"path":path}),
                &["path"],
                true,
            ),
            tool(
                "read_file",
                "Read bounded bytes as base64 in structuredContent.content. Use create_download for larger artifacts.",
                json!({"path":path,"offset":{"type":"integer","minimum":0,"default":0},"max_bytes":{"type":"integer","minimum":1,"maximum":inline}}),
                &["path"],
                true,
            ),
            tool(
                "write_file",
                "Create a new artifact from UTF-8 or standard base64. Existing objects are never overwritten.",
                json!({"path":path,"content":{"type":"string","maxLength":inline.div_ceil(3)*4},"encoding":{"type":"string","enum":["utf-8","base64"],"default":"utf-8"},"content_type":content_type}),
                &["path", "content"],
                false,
            ),
            tool(
                "create_download",
                "Create a short-lived authenticated download grant for an immutable object version. The same identity must authenticate the download.",
                json!({"path":path,"ttl_seconds":ttl}),
                &["path"],
                true,
            ),
            tool(
                "create_upload",
                "Create a bounded upload grant bound to an exact length and SHA-256. Upload raw bytes, then explicitly complete the create-only transfer.",
                json!({"path":path,"size":{"type":"integer","minimum":0,"maximum":self.storage.config().limits.transfer_bytes},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},"content_type":content_type,"ttl_seconds":ttl}),
                &["path", "size", "sha256"],
                false,
            ),
            tool(
                "complete_upload",
                "Verify and commit a staged upload without overwriting an existing object.",
                json!({"id":id}),
                &["id"],
                false,
            ),
            tool(
                "abort_upload",
                "Cancel an owned upload and remove its private staging data.",
                json!({"id":id}),
                &["id"],
                false,
            ),
        ]
    }
}

impl ServerHandler for HostedMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools_with(ToolsCapability {
                    list_changed: Some(false),
                })
                .build(),
        )
        .with_server_info(
            Implementation::new("infimount-hosted", env!("CARGO_PKG_VERSION"))
                .with_title("Infimount hosted storage"),
        )
        .with_instructions(
            "Every request requires an audience-bound OAuth access token. Paths use configured storage names. Writes are create-only. Download and upload URLs require the same authenticated identity. There are no sessions, administrative tools, overwrite operations or approval bypasses.",
        )
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<InitializeResult, ErrorData> {
        request_principal(&context)?;
        context.peer.set_peer_info(request);
        Ok(self.get_info())
    }

    async fn ping(
        &self,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<(), ErrorData> {
        request_principal(&context)?;
        Ok(())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        let principal = request_principal(&context)?;
        if request.is_some_and(|request| request.cursor.is_some()) {
            return Err(ErrorData::invalid_params("invalid cursor", None));
        }
        Ok(ListToolsResult::with_all_items(
            self.tools()
                .into_iter()
                .filter(|tool| allowed(&principal, &tool.name))
                .collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let principal = request_principal(&context).inspect_err(|_| {
            crate::audit::record(None, "mcp_request", Err(HostedError::Unauthorized), None, 0);
        })?;
        let Some(operation) = audit_operation(&request.name) else {
            crate::audit::record(
                Some(&principal),
                "unsupported_tool",
                Err(HostedError::Unsupported),
                None,
                0,
            );
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        };
        let lifetime = context
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<Arc<RequestLifetime>>())
            .ok_or_else(|| ErrorData::invalid_request("request boundary is missing", None))?;
        let path = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("path"))
            .and_then(Value::as_str)
            .filter(|path| path.len() <= 4096)
            .map(str::to_owned);
        let result = lifetime
            .run(self.dispatch(&principal, &request.name, request.arguments))
            .await;
        crate::audit::record(
            Some(&principal),
            operation,
            result.as_ref().map(|_| ()).map_err(|error| *error),
            path.as_deref(),
            0,
        );
        match result {
            Ok(result) if operation == "read_file" => {
                // The SDK's structured() convenience constructor repeats the
                // entire JSON in text content. Keep one bounded binary payload
                // and give clients a useful text summary without duplicating it.
                let summary = json!({
                    "path":result["path"],
                    "encoding":result["encoding"],
                    "read_bytes":result["read_bytes"],
                    "truncated":result["truncated"],
                    "data":"Base64 bytes are in structuredContent.content."
                });
                let mut response =
                    CallToolResult::success(vec![Content::text(summary.to_string())]);
                response.structured_content = Some(result);
                Ok(response)
            }
            Ok(result) => Ok(CallToolResult::structured(result)),
            Err(error) => Ok(CallToolResult::structured_error(
                json!({"error":error.code()}),
            )),
        }
    }
}

fn audit_operation(name: &str) -> Option<&'static str> {
    Some(match name {
        "list_dir" => "list_dir",
        "stat_path" => "stat_path",
        "read_file" => "read_file",
        "write_file" => "write_file",
        "create_download" => "create_download",
        "create_upload" => "create_upload",
        "complete_upload" => "complete_upload",
        "abort_upload" => "abort_upload",
        _ => return None,
    })
}

fn request_principal(
    context: &RequestContext<RoleServer>,
) -> std::result::Result<Principal, ErrorData> {
    let principal = context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Principal>())
        .ok_or_else(|| ErrorData::invalid_request("authentication required", None))?;
    principal
        .ensure_live()
        .map_err(|_| ErrorData::invalid_request("authentication required", None))?;
    Ok(principal.clone())
}

fn allowed(principal: &Principal, name: &str) -> bool {
    match name {
        "list_dir" | "stat_path" | "read_file" => principal.has_scope(READ_SCOPE),
        "create_download" => principal.has_scope(READ_SCOPE) && principal.has_scope(DOWNLOAD_SCOPE),
        "write_file" | "create_upload" | "complete_upload" | "abort_upload" => {
            principal.has_scope(CREATE_SCOPE)
        }
        _ => false,
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Tool {
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), properties);
    schema.insert("required".into(), json!(required));
    schema.insert("additionalProperties".into(), json!(false));
    Tool::new(name, description, Arc::new(schema)).with_annotations(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(false)
            .idempotent(read_only && name != "create_download")
            .open_world(false),
    )
}

fn decode<T: DeserializeOwned>(input: Value) -> Result<T> {
    serde_json::from_value(input).map_err(|_| HostedError::InvalidRequest)
}

fn default_limit() -> u32 {
    200
}

fn default_encoding() -> String {
    "utf-8".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathInput {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    path: String,
    #[serde(default = "default_limit")]
    limit: u32,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
    #[serde(default)]
    offset: u64,
    max_bytes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteInput {
    path: String,
    content: String,
    #[serde(default = "default_encoding")]
    encoding: String,
    content_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadInput {
    path: String,
    ttl_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadInput {
    path: String,
    size: u64,
    sha256: String,
    content_type: Option<String>,
    ttl_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferInput {
    id: String,
}
