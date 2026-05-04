use std::sync::atomic::{AtomicBool, Ordering};

static TELEMETRY_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init_telemetry() -> bool {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT");

    if endpoint.is_err() {
        return false;
    }

    let endpoint = endpoint.unwrap();
    if endpoint.is_empty() {
        return false;
    }

    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "infimount_mcp".to_string());

    TELEMETRY_ENABLED.store(true, Ordering::SeqCst);
    eprintln!(
        "[OTEL] Telemetry configured with endpoint: {}, service: {}",
        endpoint, service_name
    );
    true
}

pub fn is_enabled() -> bool {
    TELEMETRY_ENABLED.load(Ordering::SeqCst)
}

#[derive(Debug)]
pub struct TelemetryState;

impl TelemetryState {
    pub fn new() -> Self {
        Self
    }

    pub fn record_tool_call(&self, _tool_name: &str) {
        if is_enabled() {
            // OTLP metrics would be recorded here
            // Counter: mcp.tool.calls
        }
    }

    pub fn record_error(&self, _error_code: &str) {
        if is_enabled() {
            // OTLP metrics would be recorded here
            // Counter: mcp.tool.errors
        }
    }

    pub fn record_latency(&self, _tool_name: &str, _elapsed_ms: f64) {
        if is_enabled() {
            // OTLP metrics would be recorded here
            // Histogram: mcp.tool.latency.ms
        }
    }
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ToolSpan;

impl ToolSpan {
    pub fn set_attribute(&self, _key: &str, _value: impl std::fmt::Display) {}
    pub fn set_status(&self, _code: &str, _message: &str) {}
}
