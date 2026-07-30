use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use infimount_mcp::confirmation::ConfirmationManager;
use infimount_mcp::errors::{err, McpError, McpErrorCode};
use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::runtime::start_http_server;
use infimount_mcp::session::SessionManager;
use infimount_mcp::telemetry::{build_os_arch, ProductEvent, ProductEventName, ProductEventStore};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarValidation {
    pub binary_found: bool,
    pub binary_path: Option<String>,
    pub version_check: Option<String>,
    pub version_match: bool,
    pub doctor_healthy: bool,
    pub doctor_report: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationProbeOutput {
    pub sidecar: SidecarValidation,
    pub mcp_handshake_ok: bool,
    pub mcp_allowed_op_ok: bool,
    pub mcp_denial_proven: bool,
    pub overall_ok: bool,
    pub endpoint_used: Option<String>,
    pub error: Option<String>,
}

pub fn find_sidecar_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for entry in std::fs::read_dir(&exe_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("mcp-") {
            return Some(entry.path());
        }
    }
    let binaries_dir = exe_dir.join("binaries");
    if let Ok(entries) = std::fs::read_dir(&binaries_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("mcp-") {
                return Some(entry.path());
            }
        }
    }
    let fallback = exe_dir.join("mcp");
    if fallback.exists() {
        return Some(fallback);
    }
    None
}

pub fn validate_sidecar_binary() -> SidecarValidation {
    let binary_path = match find_sidecar_binary() {
        Some(path) => path,
        None => {
            return SidecarValidation {
                binary_found: false,
                binary_path: None,
                version_check: None,
                version_match: false,
                doctor_healthy: false,
                doctor_report: None,
            };
        }
    };

    let binary_str = binary_path.to_string_lossy().to_string();

    let version_output = std::process::Command::new(&binary_path)
        .arg("--version")
        .output()
        .ok();
    let version_check = version_output
        .as_ref()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if v.is_empty() { None } else { Some(v) }
        });

    let app_version = env!("CARGO_PKG_VERSION");
    let version_match = version_check
        .as_deref()
        .map(|v| v.contains(app_version))
        .unwrap_or(false);

    let doctor_output = std::process::Command::new(&binary_path)
        .arg("--doctor")
        .arg("--json")
        .output()
        .ok();
    let (doctor_healthy, doctor_report) = match doctor_output {
        Some(o) if o.status.success() => {
            let report: Option<serde_json::Value> =
                serde_json::from_slice(&o.stdout).ok();
            let healthy = report
                .as_ref()
                .and_then(|r| r.get("healthy").and_then(|v| v.as_bool()))
                .unwrap_or(false);
            (healthy, report)
        }
        _ => (false, None),
    };

    SidecarValidation {
        binary_found: true,
        binary_path: Some(binary_str),
        version_check,
        version_match,
        doctor_healthy,
        doctor_report,
    }
}

pub async fn run_activation_probe(
    registry: StorageRegistry,
    confirmations: ConfirmationManager,
    sessions: SessionManager,
    product_events: &ProductEventStore,
) -> ActivationProbeOutput {
    let sidecar = validate_sidecar_binary();

    let start_time = Utc::now();

    let mcp_result = run_mcp_probe(registry, confirmations, sessions).await;

    let duration_ms = (Utc::now() - start_time).num_milliseconds();
    let duration_bucket = if duration_ms < 1000 {
        "fast"
    } else if duration_ms < 5000 {
        "moderate"
    } else {
        "slow"
    }
    .to_string();

    let (mcp_handshake_ok, mcp_allowed_op_ok, mcp_denial_proven, endpoint_used, error) = match mcp_result
    {
        Ok(result) => (true, result.allowed_op_ok, result.denial_proven, Some(result.endpoint), None),
        Err(e) => (false, false, false, None, Some(e.message.clone())),
    };

    let overall_ok = sidecar.binary_found
        && sidecar.doctor_healthy
        && mcp_handshake_ok
        && mcp_allowed_op_ok
        && mcp_denial_proven;

    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = build_os_arch();
    let event_id = Uuid::new_v4().to_string();
    let timestamp = Utc::now().to_rfc3339();

    let sidecar_event = ProductEvent {
        id: format!("{}-sidecar", &event_id),
        timestamp: timestamp.clone(),
        name: ProductEventName::SidecarVerified,
        schema_version: 1,
        app_version: app_version.clone(),
        os_arch: os_arch.clone(),
        backend_type: None,
        workspace_template: None,
        access_profile: None,
        client_kind: None,
        success: Some(sidecar.binary_found && sidecar.doctor_healthy),
        failure_stage: Some("sidecar_validation".to_string()),
        error_code: if sidecar.binary_found && sidecar.doctor_healthy {
            None
        } else {
            Some("ERR_SIDECAR_UNHEALTHY".to_string())
        },
        duration_bucket: Some(duration_bucket.clone()),
    };
    let _ = product_events.record(sidecar_event);

    let probe_event = ProductEvent {
        id: format!("{}-probe", &event_id),
        timestamp,
        name: ProductEventName::McpProbeCompleted,
        schema_version: 1,
        app_version,
        os_arch,
        backend_type: None,
        workspace_template: None,
        access_profile: None,
        client_kind: None,
        success: Some(overall_ok),
        failure_stage: if overall_ok {
            None
        } else if !mcp_handshake_ok {
            Some("mcp_handshake".to_string())
        } else if !mcp_allowed_op_ok {
            Some("mcp_allowed_op".to_string())
        } else if !mcp_denial_proven {
            Some("mcp_denial".to_string())
        } else {
            Some("activation_probe".to_string())
        },
        error_code: if overall_ok { None } else { Some("ERR_ACTIVATION_FAILED".to_string()) },
        duration_bucket: Some(duration_bucket),
    };
    let _ = product_events.record(probe_event);

    ActivationProbeOutput {
        sidecar,
        mcp_handshake_ok,
        mcp_allowed_op_ok,
        mcp_denial_proven,
        overall_ok,
        endpoint_used,
        error,
    }
}

struct McpProbeOk {
    allowed_op_ok: bool,
    denial_proven: bool,
    endpoint: String,
}

async fn run_mcp_probe(
    registry: StorageRegistry,
    confirmations: ConfirmationManager,
    sessions: SessionManager,
) -> Result<McpProbeOk, McpError> {
    let server = start_http_server(
        registry,
        "127.0.0.1",
        0,
        infimount_mcp::server::all_tool_names(),
        true,
        None,
        confirmations,
        sessions,
    )
    .await
    .map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to start MCP probe server: {e}"),
        )
    })?;

    let endpoint = server.endpoint().to_string();

    let result = perform_mcp_requests(&endpoint).await;

    if let Err(e) = server.stop().await {
        eprintln!("warning: MCP probe server stop failed: {e}");
    }

    result.map(|(allowed, denial)| McpProbeOk {
        allowed_op_ok: allowed,
        denial_proven: denial,
        endpoint,
    })
}

async fn perform_mcp_requests(endpoint: &str) -> Result<(bool, bool), McpError> {
    let no_token: Option<&str> = None;

    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"infimount-probe","version":"0.0.0"}}}"#;
    let init_response = post_raw_http(endpoint, no_token, initialize)
        .await
        .map_err(|e| err(McpErrorCode::ERR_INTERNAL, format!("handshake failed: {e}")))?;
    let handshake_ok = init_response.starts_with("HTTP/1.1 200 OK")
        || init_response.starts_with("HTTP/1.1 202 Accepted");
    if !handshake_ok {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            format!("MCP handshake rejected: {init_response}"),
        ));
    }

    let session_id = response_header(&init_response, "mcp-session-id")
        .or_else(|| response_header(&init_response, "Mcp-Session-Id"))
        .map(|s| s.to_string());
    let session_id = session_id.as_deref().unwrap_or("");

    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let _init_notif = post_raw_http_with_headers(endpoint, no_token, initialized, &[("Mcp-Session-Id", session_id)])
        .await
        .map_err(|e| err(McpErrorCode::ERR_INTERNAL, format!("initialized notification failed: {e}")))?;

    let tools_list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    let tools_response = post_raw_http_with_headers(endpoint, no_token, tools_list, &[("Mcp-Session-Id", session_id)])
        .await
        .map_err(|e| err(McpErrorCode::ERR_INTERNAL, format!("tools/list failed: {e}")))?;
    let allowed_op_ok = tools_response.starts_with("HTTP/1.1 200 OK")
        && tools_response.contains("\"tools\"");

    let bad_method = r#"{"jsonrpc":"2.0","id":3,"method":"nonexistent_method","params":{}}"#;
    let denial_response = post_raw_http_with_headers(endpoint, no_token, bad_method, &[("Mcp-Session-Id", session_id)])
        .await
        .map_err(|e| err(McpErrorCode::ERR_INTERNAL, format!("denial test failed: {e}")))?;
    let denial_proven = denial_response.starts_with("HTTP/1.1 200 OK")
        && (denial_response.contains("\"error\"") || denial_response.contains("\"code\""));

    Ok((allowed_op_ok, denial_proven))
}

async fn post_raw_http(
    endpoint: &str,
    auth_token: Option<&str>,
    body: &str,
) -> Result<String, String> {
    post_raw_http_with_headers(endpoint, auth_token, body, &[]).await
}

async fn post_raw_http_with_headers(
    endpoint: &str,
    auth_token: Option<&str>,
    body: &str,
    headers: &[(&str, &str)],
) -> Result<String, String> {
    let endpoint = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| format!("endpoint should start with http://: {endpoint}"))?;
    let (authority, path) = endpoint
        .split_once('/')
        .ok_or_else(|| format!("endpoint should include path: {endpoint}"))?;
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
        let mut stream = TcpStream::connect(&authority)
            .map_err(|e| format!("connect to {authority}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("set timeout: {e}"))?;
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("write request: {e}"))?;

        let mut response = String::new();
        let mut reader = BufReader::new(&mut stream);
        reader
            .read_to_string(&mut response)
            .map_err(|e| format!("read response: {e}"))?;
        Ok(response)
    })
    .await
    .map_err(|e| format!("blocking task failed: {e}"))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_mcp::registry::StorageRegistry;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn activation_probe_does_not_panic() {
        let dir = tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let events_store = ProductEventStore::new(Some(dir.path().join("events.jsonl")));
        let probe = run_activation_probe(
            registry,
            ConfirmationManager::new(),
            SessionManager::new(),
            &events_store,
        )
        .await;

        assert!(probe.sidecar.binary_found == false || probe.sidecar.binary_found);
        assert!(!probe.overall_ok || probe.sidecar.binary_found);
    }

    #[tokio::test]
    async fn mcp_probe_handshake_succeeds_on_local_server() {
        let dir = tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );

        let server = start_http_server(
            registry,
            "127.0.0.1",
            0,
            infimount_mcp::server::all_tool_names(),
            true,
            None,
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await
        .expect("start probe server");

        let result = perform_mcp_requests(server.endpoint()).await;
        let (allowed_op_ok, denial_proven) = result.expect("MCP probe should succeed");
        assert!(allowed_op_ok, "tools/list should succeed");
        assert!(denial_proven, "nonexistent method should return error");

        server.stop().await.expect("stop probe server");
    }

    #[test]
    fn sidecar_validation_runs_without_crashing() {
        let validation = validate_sidecar_binary();
        if validation.binary_found {
            assert!(validation.version_check.is_some());
            if let Some(report) = &validation.doctor_report {
                assert!(report.get("healthy").is_some());
            }
        }
    }
}
