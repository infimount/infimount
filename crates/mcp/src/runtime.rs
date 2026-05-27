use std::io;
use std::net::SocketAddr;

use axum::body::Body;
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
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::confirmation::ConfirmationManager;
use crate::registry::StorageRegistry;
use crate::server::InfimountMcpServer;
use crate::session::SessionManager;
use crate::settings::{normalize_auth_token, McpSettings};
use crate::tools_fs::FsToolsContext;

pub const HTTP_ENDPOINT_PATH: &str = "/mcp";

#[derive(Clone)]
struct HttpAuthConfig {
    allow_insecure: bool,
    auth_token: Option<String>,
}

async fn enforce_http_auth(
    State(auth): State<HttpAuthConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if auth.allow_insecure {
        return next.run(request).await;
    }

    let Some(expected_token) = auth.auth_token.as_deref() else {
        return unauthorized_response();
    };

    let provided_token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    if provided_token == Some(expected_token) {
        return next.run(request).await;
    }

    unauthorized_response()
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
    let sessions = SessionManager::new();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };
    let service = InfimountMcpServer::new(ctx, enabled_tools);
    let (stdin, stdout) = rmcp::transport::stdio();
    let running = service
        .serve((stdin, stdout))
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
    let auth_token = normalize_auth_token(auth_token);

    if allow_insecure && !is_loopback_bind_address(bind_address) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unauthenticated HTTP transport is only allowed on loopback bind addresses",
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
    let config = StreamableHttpServerConfig {
        cancellation_token: cancellation_token.child_token(),
        ..Default::default()
    };
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
    start_http_server(
        registry,
        &settings.bind_address,
        settings.port,
        settings.enabled_tools.clone(),
        allow,
        effective_auth_token,
        confirmations,
        sessions,
    )
    .await
}

fn is_loopback_bind_address(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized == "localhost"
        || normalized == "::1"
        || normalized == "[::1]"
        || normalized.starts_with("127.")
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
