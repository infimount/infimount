use std::io;
use std::net::SocketAddr;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::ServiceExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::registry::StorageRegistry;
use crate::server::InfimountMcpServer;
use crate::settings::McpSettings;
use crate::tools_fs::FsToolsContext;

pub const HTTP_ENDPOINT_PATH: &str = "/mcp";

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
    let ctx = FsToolsContext { registry };
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

pub async fn start_http_server(
    registry: StorageRegistry,
    bind_address: &str,
    port: u16,
    enabled_tools: Vec<String>,
) -> io::Result<McpHttpServerHandle> {
    let cancellation_token = CancellationToken::new();
    let config = StreamableHttpServerConfig {
        cancellation_token: cancellation_token.child_token(),
        ..Default::default()
    };
    let registry_for_factory = registry.clone();
    let enabled_tools_for_factory = enabled_tools.clone();
    let service: StreamableHttpService<InfimountMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(InfimountMcpServer::new(
                    FsToolsContext {
                        registry: registry_for_factory.clone(),
                    },
                    enabled_tools_for_factory.clone(),
                ))
            },
            Default::default(),
            config,
        );

    let router = Router::new().nest_service(HTTP_ENDPOINT_PATH, service);
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
) -> io::Result<McpHttpServerHandle> {
    start_http_server(
        registry,
        &settings.bind_address,
        settings.port,
        settings.enabled_tools.clone(),
    )
    .await
}

fn display_host(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(v4) => v4.ip().to_string(),
        SocketAddr::V6(v6) => format!("[{}]", v6.ip()),
    }
}
