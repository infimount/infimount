use infimount_mcp::confirmation::ConfirmationManager;
use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::runtime::{serve_stdio, start_http_server};
use infimount_mcp::session::SessionManager;
use infimount_mcp::settings::{
    resolve_auth_token, McpSettingsStore, DEFAULT_HTTP_BIND_ADDRESS, DEFAULT_HTTP_PORT,
};
use infimount_mcp::telemetry::init_telemetry;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();

    if arg_present("--doctor") {
        let json_output = arg_present("--json");
        let report = doctor_report();
        if json_output {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        let is_healthy = report.get("healthy").and_then(|v| v.as_bool()).unwrap_or(false);
        std::process::exit(if is_healthy { 0 } else { 1 });
    }

    let _ = init_telemetry();

    let transport = arg_value("--transport").unwrap_or_else(|| "stdio".to_string());
    let allow_insecure = arg_present("--allow-insecure");
    let secret_store: std::sync::Arc<dyn infimount_core::secrets::SecretStore> =
        std::sync::Arc::new(infimount_core::secrets::NativeSecretStore::new());
    infimount_mcp::registry::retry_pending_secret_cleanup(secret_store.as_ref())
        .map_err(|error| std::io::Error::other(error.message))?;
    let registry = StorageRegistry::with_secret_store(None, secret_store.clone());
    let settings = McpSettingsStore::with_secret_store(None, secret_store.clone())
        .load()
        .map_err(|error| std::io::Error::other(error.message))?;
    let persisted_auth_token = resolve_auth_token(&settings.auth_token_ref, secret_store.as_ref())
        .map_err(|error| std::io::Error::other(error.message))?;

    let effective_auth_token = std::env::var("INFIMOUNT_AUTH_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(persisted_auth_token);
    let require_auth = effective_auth_token.is_some();
    let allow_insecure = allow_insecure && !require_auth;

    match transport.as_str() {
        "stdio" => serve_stdio(registry, settings.enabled_tools.clone())
            .await
            .map_err(|err| err as Box<dyn std::error::Error>),
        "http" => {
            let bind = arg_value("--bind").unwrap_or_else(|| DEFAULT_HTTP_BIND_ADDRESS.to_string());
            let port = arg_value("--port")
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(DEFAULT_HTTP_PORT);
            let server = start_http_server(
                registry,
                &bind,
                port,
                settings.enabled_tools.clone(),
                allow_insecure,
                effective_auth_token,
                ConfirmationManager::new(),
                SessionManager::new(),
            )
            .await?;
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

fn doctor_report() -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let config_dir = infimount_mcp::registry::default_config_dir();
    let config_dir_exists = config_dir.exists();

    let registry_path = config_dir.join("registry.json");
    let registry_exists = registry_path.exists();

    let settings_path = config_dir.join("settings.json");
    let settings_exists = settings_path.exists();

    let mut checks: Vec<serde_json::Value> = Vec::new();

    checks.push(json!({
        "name": "binary_version",
        "status": "ok",
        "value": version
    }));

    checks.push(json!({
        "name": "config_dir",
        "status": if config_dir_exists { "ok" } else { "missing" },
        "path": config_dir.to_string_lossy()
    }));

    checks.push(json!({
        "name": "registry_file",
        "status": if registry_exists { "ok" } else { "missing" },
        "path": registry_path.to_string_lossy()
    }));

    checks.push(json!({
        "name": "settings_file",
        "status": if settings_exists { "ok" } else { "missing" },
        "path": settings_path.to_string_lossy()
    }));

    let auth_env = std::env::var("INFIMOUNT_AUTH_TOKEN").ok();
    let auth_env_info = auth_env
        .as_ref()
        .map(|_| "set")
        .unwrap_or("not_set")
        .to_string();
    checks.push(json!({
        "name": "auth_env",
        "status": "ok",
        "value": auth_env_info
    }));

    let all_ok = checks.iter().all(|c| c["status"] == "ok");

    json!({
        "app": "infimount-mcp",
        "version": version,
        "os_arch": os_arch,
        "healthy": all_ok,
        "checks": checks
    })
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

fn arg_present(name: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == name)
}
