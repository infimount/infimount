use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::runtime::{serve_stdio, start_http_server};
use infimount_mcp::settings::{
    McpSettings, McpSettingsStore, DEFAULT_HTTP_BIND_ADDRESS, DEFAULT_HTTP_PORT,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let transport = arg_value("--transport").unwrap_or_else(|| "stdio".to_string());
    let registry = StorageRegistry::new(None);
    let settings = McpSettingsStore::new(None).load().unwrap_or_else(|error| {
        eprintln!(
            "failed to load MCP settings from disk (using defaults): {}",
            error.message
        );
        McpSettings::default()
    });

    match transport.as_str() {
        "stdio" => serve_stdio(registry, settings.enabled_tools.clone())
            .await
            .map_err(|err| err as Box<dyn std::error::Error>),
        "http" => {
            let bind = arg_value("--bind").unwrap_or_else(|| DEFAULT_HTTP_BIND_ADDRESS.to_string());
            let port = arg_value("--port")
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(DEFAULT_HTTP_PORT);
            let server =
                start_http_server(registry, &bind, port, settings.enabled_tools.clone()).await?;
            eprintln!(
                "Infimount MCP HTTP server listening at {}",
                server.endpoint()
            );
            tokio::signal::ctrl_c().await?;
            server.stop().await?;
            Ok(())
        }
        _ => {
            eprintln!("unsupported transport: {transport}; expected --transport stdio or http");
            std::process::exit(2);
        }
    }
}

fn arg_value(name: &str) -> Option<String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    args.windows(2).find_map(|window| {
        if window[0] == name {
            Some(window[1].clone())
        } else {
            None
        }
    })
}
