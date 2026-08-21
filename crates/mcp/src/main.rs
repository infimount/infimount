use infimount_mcp::confirmation::ConfirmationManager;
use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::runtime::{serve_stdio, start_http_server};
use infimount_mcp::session::SessionManager;
use infimount_mcp::settings::{
    resolve_auth_token, McpSettingsStore, DEFAULT_HTTP_BIND_ADDRESS, DEFAULT_HTTP_PORT,
};
use infimount_mcp::telemetry::init_telemetry;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Version,
    Doctor,
    PrintConfigDir,
    Serve(ServeArgs),
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServeArgs {
    transport: String,
    bind: String,
    port: u16,
    allow_insecure: bool,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            transport: "stdio".to_string(),
            bind: DEFAULT_HTTP_BIND_ADDRESS.to_string(),
            port: DEFAULT_HTTP_PORT,
            allow_insecure: false,
        }
    }
}

fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliCommand, String> {
    let args = args.into_iter().collect::<Vec<_>>();
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(CliCommand::Serve(ServeArgs::default()));
    };
    match command {
        "--version" if args.len() == 1 => Ok(CliCommand::Version),
        "doctor" if args.len() == 1 || (args.len() == 2 && args[1] == "--json") => {
            Ok(CliCommand::Doctor)
        }
        "print-config-dir" if args.len() == 1 => Ok(CliCommand::PrintConfigDir),
        "serve" => parse_serve_args(&args[1..]).map(CliCommand::Serve),
        "--help" | "-h" if args.len() == 1 => Ok(CliCommand::Help),
        known
            if matches!(
                known,
                "--version" | "doctor" | "print-config-dir" | "--help" | "-h"
            ) =>
        {
            Err(format!("unexpected arguments for {known}"))
        }
        unknown => Err(format!("unknown command or option: {unknown}")),
    }
}

fn parse_serve_args(args: &[String]) -> Result<ServeArgs, String> {
    let mut parsed = ServeArgs::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--transport" => {
                index += 1;
                parsed.transport = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--transport requires stdio or http".to_string())?;
                if !matches!(parsed.transport.as_str(), "stdio" | "http") {
                    return Err("--transport must be stdio or http".to_string());
                }
            }
            "--bind" => {
                index += 1;
                parsed.bind = args
                    .get(index)
                    .cloned()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "--bind requires a non-empty address".to_string())?;
            }
            "--port" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--port requires a number from 0 to 65535".to_string())?;
                parsed.port = value
                    .parse::<u16>()
                    .map_err(|_| "--port requires a number from 0 to 65535".to_string())?;
            }
            "--allow-insecure" => parsed.allow_insecure = true,
            unknown => return Err(format!("unknown serve option: {unknown}")),
        }
        index += 1;
    }
    if parsed.transport == "stdio"
        && (parsed.bind != DEFAULT_HTTP_BIND_ADDRESS
            || parsed.port != DEFAULT_HTTP_PORT
            || parsed.allow_insecure)
    {
        return Err("--bind, --port, and --allow-insecure require --transport http".to_string());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "Infimount MCP sidecar\n\nUSAGE:\n  infimount_mcp --version\n  infimount_mcp doctor --json\n  infimount_mcp print-config-dir\n  infimount_mcp serve [--transport stdio|http] [--bind ADDRESS] [--port PORT] [--allow-insecure]"
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = match parse_cli(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("run with --help for usage");
            std::process::exit(2);
        }
    };

    match command {
        CliCommand::Version => {
            println!("infimount_mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliCommand::Doctor => {
            let report = doctor_report();
            println!("{}", serde_json::to_string_pretty(&report)?);
            let is_healthy = report
                .get("healthy")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            std::process::exit(if is_healthy { 0 } else { 1 });
        }
        CliCommand::PrintConfigDir => {
            println!(
                "{}",
                infimount_mcp::registry::default_config_dir().display()
            );
            return Ok(());
        }
        CliCommand::Help => {
            print_help();
            return Ok(());
        }
        CliCommand::Serve(serve) => run_server(serve).await?,
    }
    Ok(())
}

async fn run_server(serve: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
    let _ = init_telemetry();

    let secret_store: std::sync::Arc<dyn infimount_core::secrets::SecretStore> =
        std::sync::Arc::new(infimount_core::secrets::NativeSecretStore::new());
    let registry = StorageRegistry::with_secret_store(None, secret_store.clone());
    let config_transaction = registry
        .acquire_configuration_transaction()
        .map_err(|error| std::io::Error::other(error.message))?;
    registry
        .recover_pending_imports_locked()
        .map_err(|error| std::io::Error::other(error.message))?;
    infimount_mcp::registry::retry_pending_secret_cleanup(secret_store.as_ref())
        .map_err(|error| std::io::Error::other(error.message))?;
    let storages = registry
        .load_all()
        .map_err(|error| std::io::Error::other(error.message))?;
    let settings = McpSettingsStore::with_secret_store(None, secret_store.clone())
        .load()
        .map_err(|error| std::io::Error::other(error.message))?;
    infimount_mcp::registry::recover_pending_secret_transactions(
        registry.path(),
        &storages,
        secret_store.as_ref(),
        settings.auth_token_ref.as_deref(),
    )
    .map_err(|error| std::io::Error::other(error.message))?;
    let persisted_auth_token = resolve_auth_token(&settings.auth_token_ref, secret_store.as_ref())
        .map_err(|error| std::io::Error::other(error.message))?;

    drop(config_transaction);

    let effective_auth_token = std::env::var("INFIMOUNT_AUTH_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(persisted_auth_token);
    let require_auth = effective_auth_token.is_some();
    let allow_insecure = serve.allow_insecure && !require_auth;

    match serve.transport.as_str() {
        "stdio" => serve_stdio(registry, settings.enabled_tools.clone())
            .await
            .map_err(|err| err as Box<dyn std::error::Error>),
        "http" => {
            let server = start_http_server(
                registry,
                &serve.bind,
                serve.port,
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
        _ => unreachable!("transport validated by parse_cli"),
    }
}

fn doctor_report() -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let config_dir = infimount_mcp::registry::default_config_dir();
    let config_dir_exists = config_dir.exists();

    let registry_path = infimount_mcp::registry::default_registry_path();
    let registry_status = json_file_status(&registry_path);

    let settings_path = infimount_mcp::settings::default_settings_path();
    let settings_status = json_file_status(&settings_path);

    let mut checks: Vec<serde_json::Value> = Vec::new();
    checks.push(json!({"name": "binary_version", "status": "ok", "value": version}));
    checks.push(json!({
        "name": "config_dir",
        "status": if config_dir_exists { "ok" } else { "not_initialized" }
    }));
    checks.push(json!({
        "name": "registry_file",
        "status": registry_status,
        "file": registry_path.file_name().and_then(|name| name.to_str())
    }));
    checks.push(json!({
        "name": "settings_file",
        "status": settings_status,
        "file": settings_path.file_name().and_then(|name| name.to_str())
    }));
    let auth_env_info = if std::env::var_os("INFIMOUNT_AUTH_TOKEN").is_some() {
        "set"
    } else {
        "not_set"
    };
    checks.push(json!({"name": "auth_env", "status": "ok", "value": auth_env_info}));

    let all_ok = checks.iter().all(|check| check["status"] != "error");
    json!({
        "app": "infimount-mcp",
        "version": version,
        "os_arch": os_arch,
        "healthy": all_ok,
        "checks": checks
    })
}

fn json_file_status(path: &std::path::Path) -> &'static str {
    if !path.exists() {
        return "not_configured";
    }
    match std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    {
        Some(_) => "ok",
        None => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::{json_file_status, parse_cli, CliCommand, ServeArgs};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn doctor_json_file_status_distinguishes_clean_valid_and_malformed() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("settings.json");
        assert_eq!(json_file_status(&path), "not_configured");
        std::fs::write(&path, b"{not-json").expect("write malformed fixture");
        assert_eq!(json_file_status(&path), "error");
        std::fs::write(&path, b"{}").expect("write valid fixture");
        assert_eq!(json_file_status(&path), "ok");
    }

    #[test]
    fn cli_rejects_unknown_commands_and_options() {
        assert!(parse_cli(strings(&["unknown"])).is_err());
        assert!(parse_cli(strings(&["serve", "--unknown"])).is_err());
        assert!(parse_cli(strings(&["--version", "extra"])).is_err());
        assert!(parse_cli(strings(&["--transport", "stdio"])).is_err());
        assert!(parse_cli(strings(&["serve", "--port", "invalid"])).is_err());
    }

    #[test]
    fn cli_accepts_documented_commands() {
        assert_eq!(parse_cli(strings(&["--version"])), Ok(CliCommand::Version));
        assert_eq!(
            parse_cli(strings(&["doctor", "--json"])),
            Ok(CliCommand::Doctor)
        );
        assert_eq!(
            parse_cli(strings(&["print-config-dir"])),
            Ok(CliCommand::PrintConfigDir)
        );
        assert_eq!(
            parse_cli(strings(&["serve", "--transport", "http", "--port", "0"])),
            Ok(CliCommand::Serve(ServeArgs {
                transport: "http".to_string(),
                port: 0,
                ..ServeArgs::default()
            }))
        );
    }
}
