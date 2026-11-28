use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::ServiceExt;
use serde_json::json;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::confirmation::ConfirmationManager;
use crate::registry::StorageRegistry;
use crate::server::InfimountMcpServer;
use crate::session::SessionManager;
use crate::settings::{normalize_auth_token, McpByteLimits, McpSettings};
use crate::tools_fs::FsToolsContext;

pub const HTTP_ENDPOINT_PATH: &str = "/mcp";

#[derive(Clone)]
struct HttpAuthConfig {
    allow_insecure: bool,
    auth_token: Option<String>,
    max_request_bytes: usize,
}

async fn enforce_http_auth(
    State(auth): State<HttpAuthConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !auth.allow_insecure {
        let Some(expected_token) = auth.auth_token.as_deref() else {
            return unauthorized_response();
        };
        let provided_token = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if provided_token != Some(expected_token) {
            return unauthorized_response();
        }
    }

    // The SDK collects the body itself, so an extractor's DefaultBodyLimit
    // would not apply. Count actual bytes after authentication and before the
    // SDK parses JSON, including bodies without Content-Length.
    if request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > auth.max_request_bytes as u64)
    {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let (parts, body) = request.into_parts();
    match to_bytes(body, auth.max_request_bytes).await {
        Ok(body) => next.run(Request::from_parts(parts, Body::from(body))).await,
        Err(_) => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    }
}

/// Preserve the SDK's JSON-RPC parsing/error behavior while bounding its
/// otherwise unbounded read_until allocation. Only one bounded scratch chunk
/// is read ahead, and an oversized frame terminates the input before parsing.
pub(crate) struct BoundedStdioInput<R> {
    inner: R,
    limit: usize,
    line_bytes: usize,
    failed: bool,
}

impl<R> BoundedStdioInput<R> {
    pub(crate) fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            line_bytes: 0,
            failed: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedStdioInput<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if this.failed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP request frame exceeds limit",
            )));
        }
        let mut scratch = [0_u8; 8192];
        let count = output
            .remaining()
            .min(scratch.len())
            .min(this.limit.saturating_sub(this.line_bytes) + 1);
        let mut incoming = ReadBuf::new(&mut scratch[..count]);
        std::task::ready!(Pin::new(&mut this.inner).poll_read(cx, &mut incoming))?;
        for byte in incoming.filled() {
            if *byte == b'\n' {
                this.line_bytes = 0;
            } else {
                this.line_bytes += 1;
                // The SDK strips an optional CR before LF (or EOF). Permit
                // that one framing byte, but no following non-LF byte.
                if this.line_bytes > this.limit
                    && !(this.line_bytes == this.limit + 1 && *byte == b'\r')
                {
                    this.failed = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP request frame exceeds limit",
                    )));
                }
            }
        }
        output.put_slice(incoming.filled());
        Poll::Ready(Ok(()))
    }
}

fn unauthorized_response() -> Response {
    let body = json!({
        "ok": false,
        "error": {
            "code": "ERR_UNAUTHORIZED",
            "message": "unauthorized: missing or invalid bearer token",
            "details": {}
        }
    });
    (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response()
}

pub struct McpHttpServerHandle {
    endpoint: String,
    cancellation_token: CancellationToken,
    join_handle: JoinHandle<io::Result<()>>,
}

impl McpHttpServerHandle {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn stop(self) -> io::Result<()> {
        self.cancellation_token.cancel();
        match self.join_handle.await {
            Ok(result) => result,
            Err(err) => Err(io::Error::other(format!(
                "MCP HTTP server task failed: {err}"
            ))),
        }
    }
}

pub async fn serve_stdio(
    registry: StorageRegistry,
    enabled_tools: Vec<String>,
) -> Result<(), Box<rmcp::RmcpError>> {
    serve_stdio_with_limits(registry, enabled_tools, McpByteLimits::default()).await
}

pub async fn serve_stdio_with_limits(
    registry: StorageRegistry,
    enabled_tools: Vec<String>,
    byte_limits: McpByteLimits,
) -> Result<(), Box<rmcp::RmcpError>> {
    byte_limits
        .validate()
        .map_err(|error| Box::new(rmcp::RmcpError::TaskError(error.message)))?;
    let request_limit = byte_limits.max_request_bytes();
    let sessions = SessionManager::new();
    let ctx = FsToolsContext {
        byte_limits,
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };
    let service = InfimountMcpServer::new(ctx, enabled_tools);
    let (stdin, stdout) = rmcp::transport::stdio();
    let running = service
        .serve((BoundedStdioInput::new(stdin, request_limit), stdout))
        .await
        .map_err(rmcp::RmcpError::from)
        .map_err(Box::new)?;
    let _ = running
        .waiting()
        .await
        .map_err(rmcp::RmcpError::from)
        .map_err(Box::new)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn start_http_server(
    registry: StorageRegistry,
    bind_address: &str,
    port: u16,
    enabled_tools: Vec<String>,
    allow_insecure: bool,
    auth_token: Option<String>,
    confirmations: ConfirmationManager,
    sessions: SessionManager,
) -> io::Result<McpHttpServerHandle> {
    start_http_server_with_limits(
        registry,
        bind_address,
        port,
        enabled_tools,
        allow_insecure,
        auth_token,
        confirmations,
        sessions,
        McpByteLimits::default(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn start_http_server_with_limits(
    registry: StorageRegistry,
    bind_address: &str,
    port: u16,
    enabled_tools: Vec<String>,
    allow_insecure: bool,
    auth_token: Option<String>,
    confirmations: ConfirmationManager,
    sessions: SessionManager,
    byte_limits: McpByteLimits,
) -> io::Result<McpHttpServerHandle> {
    byte_limits
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.message))?;
    let auth_token = normalize_auth_token(auth_token);

    if !is_loopback_bind_address(bind_address) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "built-in MCP HTTP transport is loopback-only; use a TLS reverse proxy for remote access",
        ));
    }

    if allow_insecure {
        eprintln!("[WARNING] HTTP server running in INSECURE mode (no authentication). Use only for local development.");
    }

    if !allow_insecure && auth_token.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "HTTP transport requires INFIMOUNT_AUTH_TOKEN or --allow-insecure",
        ));
    }

    let cancellation_token = CancellationToken::new();
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(cancellation_token.child_token());
    let registry_for_factory = registry.clone();
    let enabled_tools_for_factory = enabled_tools.clone();
    let allow_insecure_for_factory = allow_insecure;
    let auth_token_for_factory = auth_token.clone();
    let sessions_for_factory = sessions.clone();
    let confirmations_for_factory = confirmations.clone();
    let service: StreamableHttpService<InfimountMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(InfimountMcpServer::with_confirmation_manager(
                    FsToolsContext {
                        byte_limits,
                        registry: registry_for_factory.clone(),
                        sessions: sessions_for_factory.clone(),
                        allow_insecure: allow_insecure_for_factory,
                        auth_token: auth_token_for_factory.clone(),
                    },
                    enabled_tools_for_factory.clone(),
                    confirmations_for_factory.clone(),
                ))
            },
            Default::default(),
            config,
        );

    let auth = HttpAuthConfig {
        allow_insecure,
        auth_token: auth_token.clone(),
        max_request_bytes: byte_limits.max_request_bytes(),
    };

    let router = Router::new()
        .nest_service(HTTP_ENDPOINT_PATH, service)
        .layer(from_fn_with_state(auth, enforce_http_auth));
    let listener = tokio::net::TcpListener::bind(format!("{bind_address}:{port}")).await?;
    let addr = listener.local_addr()?;
    let endpoint = format!(
        "http://{}:{}{}",
        display_host(addr),
        addr.port(),
        HTTP_ENDPOINT_PATH
    );
    let shutdown = cancellation_token.clone();

    let join_handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown.cancelled_owned().await;
            })
            .await
            .map_err(io::Error::other)
    });

    Ok(McpHttpServerHandle {
        endpoint,
        cancellation_token,
        join_handle,
    })
}

pub async fn start_http_server_from_settings(
    registry: StorageRegistry,
    settings: &McpSettings,
    allow_insecure: bool,
    confirmations: ConfirmationManager,
    sessions: SessionManager,
) -> io::Result<McpHttpServerHandle> {
    let effective_auth_token = settings.auth_token.clone();
    let require_auth = effective_auth_token.is_some();
    let allow = allow_insecure && !require_auth;
    start_http_server_with_limits(
        registry,
        &settings.bind_address,
        settings.port,
        settings.enabled_tools.clone(),
        allow,
        effective_auth_token,
        confirmations,
        sessions,
        settings.byte_limits(),
    )
    .await
}

pub fn is_loopback_bind_address(value: &str) -> bool {
    let value = value.trim();
    if value.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let unbracketed = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    unbracketed
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

fn display_host(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(v4) => v4.ip().to_string(),
        SocketAddr::V6(v6) => format!("[{}]", v6.ip()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::server::all_tool_names;

    fn temp_registry(temp_dir: &TempDir) -> StorageRegistry {
        StorageRegistry::new(Some(temp_dir.path().join("storages.json")))
    }

    async fn post_raw_http(
        endpoint: &str,
        auth_token: Option<&str>,
        body: &str,
    ) -> io::Result<String> {
        post_raw_http_with_headers(endpoint, auth_token, body, &[]).await
    }

    async fn post_raw_http_with_headers(
        endpoint: &str,
        auth_token: Option<&str>,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<String> {
        let endpoint = endpoint
            .strip_prefix("http://")
            .expect("test endpoint should be http");
        let (authority, path) = endpoint
            .split_once('/')
            .expect("test endpoint should include path");
        let path = format!("/{path}");
        let mut request = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {authority}\r\n\
             Content-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n",
            body.len()
        );
        if let Some(token) = auth_token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);

        let authority = authority.to_string();
        tokio::task::spawn_blocking(move || {
            let mut stream = TcpStream::connect(authority)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.write_all(request.as_bytes())?;

            let mut response = String::new();
            match stream.read_to_string(&mut response) {
                Ok(_) => Ok(response),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    Ok(response)
                }
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| io::Error::other(format!("HTTP request task failed: {error}")))?
    }

    fn response_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
        response
            .lines()
            .take_while(|line| !line.is_empty() && *line != "\r")
            .find_map(|line| {
                let (header_name, value) = line.split_once(':')?;
                header_name.eq_ignore_ascii_case(name).then(|| value.trim())
            })
    }

    #[tokio::test]
    async fn stdio_input_bounds_each_frame_before_sdk_collection_and_resets_at_newline() {
        use tokio::io::AsyncBufReadExt;
        let limits = McpByteLimits {
            max_read_bytes: 2,
            max_write_bytes: 3,
        };
        let limit = limits.max_request_bytes();
        let frame = [vec![b' '; limit], vec![b'\n']].concat();
        let input = [frame.clone(), frame.clone()].concat();
        let mut reader = tokio::io::BufReader::new(BoundedStdioInput::new(input.as_slice(), limit));
        for _ in 0..2 {
            let mut message = Vec::new();
            assert_eq!(
                reader.read_until(b'\n', &mut message).await.unwrap(),
                limit + 1
            );
            assert_eq!(message, frame);
        }
        let oversized = vec![b' '; limit + 1];
        let mut reader =
            tokio::io::BufReader::new(BoundedStdioInput::new(oversized.as_slice(), limit));
        let mut message = Vec::new();
        let error = reader.read_until(b'\n', &mut message).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(message.len() <= limit);
    }

    #[tokio::test]
    async fn stdio_limit_preserves_fragmented_crlf_and_a_valid_frame_before_rejection() {
        use tokio::io::AsyncBufReadExt;
        let limit = 64;
        for ending in [b"\n".as_slice(), b"\r\n", b"\r"] {
            let input = [vec![b' '; limit], ending.to_vec()].concat();
            let mut reader = tokio::io::BufReader::with_capacity(
                1,
                BoundedStdioInput::new(input.as_slice(), limit),
            );
            let mut received = Vec::new();
            reader.read_until(b'\n', &mut received).await.unwrap();
            assert_eq!(received, input);
        }
        let valid = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n";
        let input = [valid.to_vec(), vec![b'x'; limit + 1]].concat();
        let mut reader = tokio::io::BufReader::new(BoundedStdioInput::new(input.as_slice(), limit));
        let mut received = Vec::new();
        reader.read_until(b'\n', &mut received).await.unwrap();
        assert_eq!(received, valid);
        received.clear();
        assert_eq!(
            reader
                .read_until(b'\n', &mut received)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(received.len() <= limit);
    }

    #[tokio::test]
    async fn http_request_limit_covers_chunked_bodies_before_sdk_parsing_or_approval() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = TempDir::new().unwrap();
        let limits = McpByteLimits {
            max_read_bytes: 2,
            max_write_bytes: 3,
        };
        let confirmations = ConfirmationManager::new();
        let server = start_http_server_with_limits(
            temp_registry(&dir),
            "127.0.0.1",
            0,
            vec!["write_file".into()],
            false,
            Some("binary-test-token".into()),
            confirmations.clone(),
            SessionManager::new(),
            limits,
        )
        .await
        .unwrap();
        let endpoint = reqwest::Url::parse(server.endpoint()).unwrap();
        let address = format!("127.0.0.1:{}", endpoint.port().unwrap());
        // Authenticate before even reading an advertised oversized body.
        let mut unauthenticated = tokio::net::TcpStream::connect(&address).await.unwrap();
        unauthenticated.write_all(format!(
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            limits.max_request_bytes() + 1,
        ).as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            unauthenticated.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 401"));

        let mut socket = tokio::net::TcpStream::connect(&address).await.unwrap();
        let body = " ".repeat(limits.max_request_bytes() + 1);
        socket.write_all(format!(
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer binary-test-token\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len(),
        ).as_bytes()).await.unwrap();
        response.clear();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 413"));
        assert!(confirmations.list_pending().await.is_empty());
        server.stop().await.unwrap();
    }

    async fn post_mcp_rpc(
        client: &reqwest::Client,
        endpoint: &str,
        session: Option<&str>,
        message: serde_json::Value,
    ) -> (reqwest::header::HeaderMap, serde_json::Value) {
        let expected_id = message["id"].clone();
        let mut request = client
            .post(endpoint)
            .bearer_auth("binary-test-token")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .header("Connection", "close")
            .json(&message);
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        let mut response = request.send().await.unwrap();
        assert!(response.status().is_success(), "HTTP {}", response.status());
        let headers = response.headers().clone();
        let is_sse = headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/event-stream");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            bytes.extend_from_slice(&chunk);
            if is_sse {
                for line in bytes.split(|byte| *byte == b'\n') {
                    if let Some(data) = line
                        .strip_prefix(b"data: ")
                        .or_else(|| line.strip_prefix(b"data:"))
                    {
                        if let Ok(reply) = serde_json::from_slice::<serde_json::Value>(data) {
                            if reply["id"] == expected_id {
                                return (headers, reply);
                            }
                        }
                    }
                }
            } else if let Ok(reply) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                assert_eq!(reply["id"], expected_id);
                return (headers, reply);
            }
        }
        panic!("MCP response ended without the requested JSON-RPC reply");
    }

    #[tokio::test]
    async fn real_http_binary_round_trip_honors_persisted_limits_confirmation_and_create_only() {
        use crate::policy::McpAccessMode;
        use crate::registry::StorageRecord;
        use crate::settings::{McpSettingsStore, McpTransport};
        use base64::Engine;
        use sha2::{Digest, Sha256};

        for limits in [
            McpByteLimits {
                max_read_bytes: 2,
                max_write_bytes: 3,
            },
            McpByteLimits::default(),
        ] {
            let dir = TempDir::new().unwrap();
            let root = dir.path().join("root");
            std::fs::create_dir_all(&root).unwrap();
            let registry = temp_registry(&dir);
            let mut storage =
                StorageRecord::new("Local".into(), "local".into(), json!({"root": root}));
            storage.mcp_exposed = true;
            storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
            registry.save_all_atomic(&[storage]).unwrap();
            let settings_store = McpSettingsStore::new(Some(dir.path().join("mcp_settings.json")));
            settings_store
                .save_atomic(&McpSettings {
                    enabled: true,
                    transport: McpTransport::Http,
                    max_read_bytes: limits.max_read_bytes,
                    max_write_bytes: limits.max_write_bytes,
                    enabled_tools: all_tool_names(),
                    ..McpSettings::default()
                })
                .unwrap();
            let mut settings = settings_store.load().unwrap();
            assert_eq!(settings.byte_limits(), limits);
            settings.port = 0;
            settings.auth_token = Some("binary-test-token".into());
            let confirmations = ConfirmationManager::new();
            let server = start_http_server_from_settings(
                registry,
                &settings,
                false,
                confirmations.clone(),
                SessionManager::new(),
            )
            .await
            .unwrap();
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap();
            let (headers, initialized) = post_mcp_rpc(
                &client,
                server.endpoint(),
                None,
                json!({
                    "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                        "protocolVersion":"2025-06-18", "capabilities":{},
                        "clientInfo":{"name":"binary-artifact-qualification","version":"1"}
                    }
                }),
            )
            .await;
            assert!(initialized.get("error").is_none());
            let session = headers
                .get("mcp-session-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let notification = client
                .post(server.endpoint())
                .bearer_auth("binary-test-token")
                .header("Accept", "application/json, text/event-stream")
                .header("Mcp-Session-Id", &session)
                .header("MCP-Protocol-Version", "2025-06-18")
                .header("Connection", "close")
                .json(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
                .send()
                .await
                .unwrap();
            assert!(notification.status().is_success());
            drop(notification);
            let (_, listed) = post_mcp_rpc(
                &client,
                server.endpoint(),
                Some(&session),
                json!({
                    "jsonrpc":"2.0", "id":2, "method":"tools/list", "params":{}
                }),
            )
            .await;
            let read_schema = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == "read_file")
                .unwrap();
            assert_eq!(
                read_schema["inputSchema"]["properties"]["max_bytes"]["maximum"],
                limits.max_read_bytes
            );

            let bytes = vec![0xab; limits.max_write_bytes as usize];
            let expected_hash = Sha256::digest(&bytes);
            let mut arguments = json!({
                "path":"/Local/artifact.bin", "content":base64::engine::general_purpose::STANDARD.encode(&bytes),
                "encoding":"base64", "overwrite":false
            });
            let envelope = json!({"jsonrpc":"2.0", "id":3, "method":"tools/call", "params":{"name":"write_file","arguments":arguments}});
            // A valid four-MiB binary artifact exceeds four MiB on the wire.
            assert!(serde_json::to_vec(&envelope).unwrap().len() > bytes.len());
            let (_, pending) =
                post_mcp_rpc(&client, server.endpoint(), Some(&session), envelope).await;
            let pending_data = &pending["result"]["structuredContent"]["data"];
            assert_eq!(pending_data["status"], "requires_confirmation");
            assert!(!root.join("artifact.bin").exists());
            let approval = pending_data["operation_id"].as_str().unwrap();
            confirmations.approve(approval).await.unwrap();
            arguments["confirmation_id"] = json!(approval);
            let (_, written) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                "jsonrpc":"2.0", "id":4, "method":"tools/call", "params":{"name":"write_file","arguments":arguments}
            })).await;
            assert_eq!(
                written["result"]["structuredContent"]["data"]["written_bytes"],
                bytes.len()
            );
            assert_eq!(
                Sha256::digest(std::fs::read(root.join("artifact.bin")).unwrap()),
                expected_hash
            );

            let mut read_hash = Sha256::new();
            let mut offset = 0usize;
            let mut id = 5u64;
            while offset < bytes.len() {
                let (_, read) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                    "jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"read_file","arguments":{
                        "path":"/Local/artifact.bin", "as_text":false, "offset_bytes":offset, "max_bytes":limits.max_read_bytes
                    }}
                })).await;
                let data = &read["result"]["structuredContent"]["data"];
                let chunk = base64::engine::general_purpose::STANDARD
                    .decode(data["content"].as_str().unwrap())
                    .unwrap();
                assert_eq!(
                    chunk.len(),
                    (bytes.len() - offset).min(limits.max_read_bytes as usize)
                );
                assert_eq!(data["read_bytes"], chunk.len());
                offset += chunk.len();
                assert_eq!(data["truncated"], offset < bytes.len());
                read_hash.update(chunk);
                id += 1;
            }
            assert_eq!(read_hash.finalize(), expected_hash);
            let (_, over_read) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"read_file","arguments":{
                    "path":"/Local/artifact.bin", "max_bytes":limits.max_read_bytes + 1
                }}
            })).await;
            assert_eq!(
                over_read["result"]["structuredContent"]["error"]["code"],
                "ERR_INVALID_PATH"
            );
            id += 1;
            let (_, over_write) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"write_file","arguments":{
                    "path":"/Local/missing/over.bin", "content":base64::engine::general_purpose::STANDARD.encode(vec![0xab; bytes.len() + 1]),
                    "encoding":"base64", "overwrite":false, "create_parents":true
                }}
            })).await;
            assert_eq!(
                over_write["result"]["structuredContent"]["error"]["code"],
                "ERR_INVALID_PATH"
            );
            assert!(confirmations.list_pending().await.is_empty());
            assert!(!root.join("missing").exists());

            // A second approved create still cannot replace existing bytes.
            id += 1;
            let mut collision_args = json!({"path":"/Local/artifact.bin","content":"AAAA","encoding":"base64","overwrite":false});
            let (_, pending) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"write_file","arguments":collision_args}
            })).await;
            let approval = pending["result"]["structuredContent"]["data"]["operation_id"]
                .as_str()
                .unwrap();
            confirmations.approve(approval).await.unwrap();
            collision_args["confirmation_id"] = json!(approval);
            id += 1;
            let (_, collision) = post_mcp_rpc(&client, server.endpoint(), Some(&session), json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"write_file","arguments":collision_args}
            })).await;
            assert_eq!(
                collision["result"]["structuredContent"]["error"]["code"],
                "ERR_ALREADY_EXISTS"
            );
            assert_eq!(
                Sha256::digest(std::fs::read(root.join("artifact.bin")).unwrap()),
                expected_hash
            );
            drop(client);
            server.stop().await.unwrap();
        }
    }

    #[test]
    fn loopback_parser_rejects_hostnames_that_only_start_with_127() {
        for address in [
            "127.0.0.1",
            "127.255.255.255",
            "::1",
            "[::1]",
            "localhost",
            "LOCALHOST",
        ] {
            assert!(is_loopback_bind_address(address), "{address}");
        }

        for address in [
            "127.example.com",
            "127.0.0.1.example.com",
            "0.0.0.0",
            "192.168.1.2",
            "::",
            "example.com",
        ] {
            assert!(!is_loopback_bind_address(address), "{address}");
        }
    }

    #[tokio::test]
    async fn http_server_requires_auth_without_insecure_override() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let result = start_http_server(
            temp_registry(&temp_dir),
            "127.0.0.1",
            0,
            all_tool_names(),
            false,
            None,
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await;

        let error = match result {
            Ok(server) => {
                let _ = server.stop().await;
                panic!("server should reject missing auth token");
            }
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("INFIMOUNT_AUTH_TOKEN"));
    }

    #[tokio::test]
    async fn http_server_rejects_authenticated_non_loopback_bind() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let result = start_http_server(
            temp_registry(&temp_dir),
            "0.0.0.0",
            0,
            all_tool_names(),
            false,
            Some("test-token".to_string()),
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await;

        let error = match result {
            Ok(server) => {
                let _ = server.stop().await;
                panic!("server should reject remote plaintext HTTP");
            }
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("loopback-only"));
    }

    #[tokio::test]
    async fn http_server_rejects_insecure_non_loopback_bind() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let result = start_http_server(
            temp_registry(&temp_dir),
            "0.0.0.0",
            0,
            all_tool_names(),
            true,
            None,
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await;

        let error = match result {
            Ok(server) => {
                let _ = server.stop().await;
                panic!("server should reject unauthenticated non-loopback bind");
            }
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("loopback"));
    }

    #[tokio::test]
    async fn http_server_enforces_bearer_auth_over_real_http() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = start_http_server(
            temp_registry(&temp_dir),
            "127.0.0.1",
            0,
            all_tool_names(),
            false,
            Some("test-token".to_string()),
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await
        .expect("start authenticated test server");

        let missing_auth = post_raw_http(server.endpoint(), None, "{}")
            .await
            .expect("missing-auth request should complete");
        assert!(missing_auth.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(missing_auth.contains("ERR_UNAUTHORIZED"));

        let wrong_auth = post_raw_http(server.endpoint(), Some("wrong-token"), "{}")
            .await
            .expect("wrong-auth request should complete");
        assert!(wrong_auth.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(wrong_auth.contains("ERR_UNAUTHORIZED"));

        let valid_auth = post_raw_http(server.endpoint(), Some("test-token"), "{}")
            .await
            .expect("valid-auth request should reach MCP transport");
        assert!(
            !valid_auth.starts_with("HTTP/1.1 401 Unauthorized"),
            "valid bearer token should pass middleware: {valid_auth}"
        );

        server.stop().await.expect("stop test server");
    }

    #[tokio::test]
    async fn http_server_completes_initialize_and_lists_tools_over_real_http() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = start_http_server(
            temp_registry(&temp_dir),
            "127.0.0.1",
            0,
            vec!["list_dir".to_string(), "stat_path".to_string()],
            false,
            Some("test-token".to_string()),
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await
        .expect("start authenticated test server");

        let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"infimount-test","version":"0.0.0"}}}"#;
        let initialize_response = post_raw_http(server.endpoint(), Some("test-token"), initialize)
            .await
            .expect("initialize request should complete");
        assert!(initialize_response.starts_with("HTTP/1.1 200 OK"));
        assert!(initialize_response.contains("\"protocolVersion\""));
        assert!(initialize_response.contains("\"serverInfo\""));

        let session_id = response_header(&initialize_response, "mcp-session-id")
            .expect("initialize should return mcp-session-id")
            .to_string();

        let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let initialized_response = post_raw_http_with_headers(
            server.endpoint(),
            Some("test-token"),
            initialized,
            &[("Mcp-Session-Id", &session_id)],
        )
        .await
        .expect("initialized notification should complete");
        assert!(
            initialized_response.starts_with("HTTP/1.1 202 Accepted")
                || initialized_response.starts_with("HTTP/1.1 200 OK"),
            "initialized notification response should be accepted: {initialized_response}"
        );

        let tools_list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let tools_response = post_raw_http_with_headers(
            server.endpoint(),
            Some("test-token"),
            tools_list,
            &[("Mcp-Session-Id", &session_id)],
        )
        .await
        .expect("tools/list request should complete");
        assert!(tools_response.starts_with("HTTP/1.1 200 OK"));
        assert!(tools_response.contains("\"tools\""));
        assert!(tools_response.contains("\"list_dir\""));
        assert!(tools_response.contains("\"stat_path\""));
        assert!(!tools_response.contains("\"read_file\""));

        server.stop().await.expect("stop test server");
    }

    #[tokio::test]
    async fn http_server_allows_port_zero_and_reports_actual_endpoint_in_insecure_mode() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let server = start_http_server(
            temp_registry(&temp_dir),
            "127.0.0.1",
            0,
            all_tool_names(),
            true,
            None,
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await
        .expect("start insecure test server");

        assert!(server.endpoint().starts_with("http://127.0.0.1:"));
        assert!(server.endpoint().ends_with(HTTP_ENDPOINT_PATH));

        server.stop().await.expect("stop test server");
    }

    #[tokio::test]
    async fn http_server_rejects_whitespace_auth_token_as_missing() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let result = start_http_server(
            temp_registry(&temp_dir),
            "127.0.0.1",
            0,
            all_tool_names(),
            false,
            Some("   ".to_string()),
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await;

        let error = match result {
            Ok(server) => {
                let _ = server.stop().await;
                panic!("server should reject whitespace auth token");
            }
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
