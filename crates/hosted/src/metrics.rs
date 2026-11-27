//! Fixed-cardinality, process-lifetime counters derived from audit events.
//! Identity, paths, tokens, transfer IDs and arbitrary labels are never accepted.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;

const OPERATIONS: [&str; 21] = [
    "http_target",
    "http_headers",
    "http_public_request",
    "http_concurrency",
    "http_authentication",
    "http_request",
    "mcp_request",
    "list_dir",
    "stat_path",
    "read_file",
    "write_file",
    "create_download",
    "delegate_download",
    "create_upload",
    "complete_upload",
    "abort_upload",
    "artifact_upload",
    "artifact_commit",
    "artifact_abort",
    "artifact_download",
    "other",
];
const OUTCOMES: [&str; 2] = ["allowed", "denied"];
const COUNTER_COUNT: usize = OPERATIONS.len() * OUTCOMES.len();
const REPORT_PERIOD: Duration = Duration::from_secs(30);

static COUNTERS: Counters = Counters::new();

struct Counters {
    start_time_unix_nano: OnceLock<u64>,
    events: [AtomicU64; COUNTER_COUNT],
    bytes: [AtomicU64; COUNTER_COUNT],
}

#[derive(Serialize)]
pub(crate) struct Snapshot {
    event: &'static str,
    timestamp_unix_seconds: u64,
    counters: Vec<CounterSnapshot>,
    #[serde(skip)]
    start_time_unix_nano: u64,
    #[serde(skip)]
    time_unix_nano: u64,
}

#[derive(Serialize)]
struct CounterSnapshot {
    operation: &'static str,
    outcome: &'static str,
    audit_events_total: u64,
    reported_bytes_total: u64,
}

impl Counters {
    const fn new() -> Self {
        Self {
            start_time_unix_nano: OnceLock::new(),
            events: [const { AtomicU64::new(0) }; COUNTER_COUNT],
            bytes: [const { AtomicU64::new(0) }; COUNTER_COUNT],
        }
    }

    fn record(&self, operation: &str, allowed: bool, bytes: u64) {
        self.start_time_unix_nano.get_or_init(now_nanos);
        let operation = OPERATIONS
            .iter()
            .position(|candidate| *candidate == operation)
            .unwrap_or(OPERATIONS.len() - 1);
        let outcome = usize::from(!allowed);
        let index = operation * OUTCOMES.len() + outcome;
        saturating_add(&self.events[index], 1);
        saturating_add(&self.bytes[index], bytes);
    }

    fn snapshot(&self) -> Snapshot {
        let start_time_unix_nano = *self.start_time_unix_nano.get_or_init(now_nanos);
        let mut counters = Vec::with_capacity(COUNTER_COUNT);
        for (operation_index, operation) in OPERATIONS.iter().enumerate() {
            for (outcome_index, outcome) in OUTCOMES.iter().enumerate() {
                let index = operation_index * OUTCOMES.len() + outcome_index;
                counters.push(CounterSnapshot {
                    operation,
                    outcome,
                    audit_events_total: self.events[index].load(Ordering::Relaxed),
                    reported_bytes_total: self.bytes[index].load(Ordering::Relaxed),
                });
            }
        }
        Snapshot {
            event: "hosted_metrics",
            timestamp_unix_seconds: crate::now_epoch_seconds(),
            counters,
            start_time_unix_nano,
            time_unix_nano: now_nanos().max(start_time_unix_nano),
        }
    }
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

impl Snapshot {
    /// OTLP/HTTP JSON uses decimal strings for 64-bit values. Counter points
    /// have one process-lifetime start time and cumulative temporality.
    pub(crate) fn otlp_payload(&self) -> serde_json::Value {
        use serde_json::json;
        let points = |bytes: bool| {
            self.counters
                .iter()
                .map(|row| {
                    let value = if bytes {
                        row.reported_bytes_total
                    } else {
                        row.audit_events_total
                    };
                    json!({
                        "attributes": [
                            {"key": "operation", "value": {"stringValue": row.operation}},
                            {"key": "outcome", "value": {"stringValue": row.outcome}}
                        ],
                        "startTimeUnixNano": self.start_time_unix_nano.to_string(),
                        "timeUnixNano": self.time_unix_nano.to_string(),
                        "asInt": value.min(i64::MAX as u64).to_string()
                    })
                })
                .collect::<Vec<_>>()
        };
        json!({
            "resourceMetrics": [{
                "resource": {"attributes": [
                    {"key": "service.name", "value": {"stringValue": "infimount-hosted"}},
                    {"key": "service.version", "value": {"stringValue": env!("CARGO_PKG_VERSION")}}
                ]},
                "scopeMetrics": [{
                    "scope": {"name": "infimount_hosted.metrics", "version": "1"},
                    "metrics": [
                        {"name": "infimount.hosted.audit.events", "unit": "1",
                         "description": "Audit events by fixed operation and outcome",
                         "sum": {"aggregationTemporality": 2, "isMonotonic": true, "dataPoints": points(false)}},
                        {"name": "infimount.hosted.reported.bytes", "unit": "By",
                         "description": "Bytes reported by audit events, not unique transferred bytes",
                         "sum": {"aggregationTemporality": 2, "isMonotonic": true, "dataPoints": points(true)}}
                    ]
                }]
            }]
        })
    }
}

fn saturating_add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

pub(crate) fn record(operation: &str, allowed: bool, bytes: u64) {
    COUNTERS.record(operation, allowed, bytes);
}

pub(crate) fn snapshot() -> Snapshot {
    COUNTERS.snapshot()
}

pub(crate) struct Reporter {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Reporter {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Reporting only exists while `serve` owns this guard. Construction/checks
/// never start a task. OTLP export is independent of request handling and JSON
/// logging, with one in-flight snapshot and no event queue or retries.
pub(crate) fn start_reporting(exporter: Option<crate::telemetry::OtlpExporter>) -> Reporter {
    let mut tasks = vec![tokio::spawn(async {
        let mut interval =
            tokio::time::interval_at(tokio::time::Instant::now() + REPORT_PERIOD, REPORT_PERIOD);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Ok(snapshot) = serde_json::to_string(&snapshot()) {
                tracing::info!(target: "infimount_hosted::metrics", snapshot = %snapshot);
            }
        }
    })];
    if let Some(exporter) = exporter {
        tasks.push(tokio::spawn(async move {
            let period = exporter.interval();
            let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(code) = exporter.export(&snapshot()).await {
                    // Exporter errors are static codes. Never log endpoints,
                    // credentials, or arbitrary collector response text.
                    tracing::warn!(target: "infimount_hosted::telemetry", event = "otlp_export_failed", code);
                }
            }
        }));
    }
    Reporter { tasks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn snapshots_have_fixed_labels_and_no_identity_path_or_token_fields() {
        let counters = Counters::new();
        let untrusted_operation =
            "principal=alice;path=/private/file;token=never-copy-this;grant=secret";
        for index in 0..1000 {
            counters.record(&format!("{untrusted_operation}-{index}"), false, 3);
        }
        counters.record("read_file", true, 4096);
        let snapshot = serde_json::to_value(counters.snapshot()).unwrap();
        let rows = snapshot["counters"].as_array().unwrap();
        assert_eq!(rows.len(), COUNTER_COUNT);
        let other = rows
            .iter()
            .find(|row| row["operation"] == "other" && row["outcome"] == "denied")
            .unwrap();
        assert_eq!(other["audit_events_total"], 1000);
        assert_eq!(other["reported_bytes_total"], 3000);
        assert!(!snapshot.to_string().contains("never-copy-this"));
        assert!(!snapshot.to_string().contains("alice"));
        assert!(!snapshot.to_string().contains("/private/file"));
        fn assert_fields(value: &Value) {
            match value {
                Value::Object(object) => {
                    for (field, value) in object {
                        assert!(![
                            "principal",
                            "issuer",
                            "subject",
                            "tenant",
                            "tenant_id",
                            "client_id",
                            "path",
                            "path_sha256",
                            "token",
                            "authorization",
                            "grant",
                            "grant_id",
                            "event_id",
                        ]
                        .contains(&field.as_str()));
                        assert_fields(value);
                    }
                }
                Value::Array(values) => values.iter().for_each(assert_fields),
                _ => {}
            }
        }
        assert_fields(&snapshot);
    }

    #[test]
    fn snapshots_do_not_reset_counters_and_totals_saturate_instead_of_wrapping() {
        let counters = Counters::new();
        counters.record("artifact_download", true, u64::MAX);
        let first = counters.snapshot();
        counters.record("artifact_download", true, 10);
        let second = counters.snapshot();
        let find = |snapshot: Snapshot| {
            snapshot
                .counters
                .into_iter()
                .find(|row| row.operation == "artifact_download" && row.outcome == "allowed")
                .unwrap()
        };
        assert_eq!(find(first).audit_events_total, 1);
        let second = find(second);
        assert_eq!(second.audit_events_total, 2);
        assert_eq!(second.reported_bytes_total, u64::MAX);
        assert_eq!(find(counters.snapshot()).audit_events_total, 2);
    }

    #[test]
    fn otlp_preserves_start_time_fixed_cardinality_and_signed_integer_range() {
        let counters = Counters::new();
        counters.record("secret-path-and-owner", true, u64::MAX);
        let first = counters.snapshot().otlp_payload();
        counters.record("read_file", true, 7);
        let second = counters.snapshot().otlp_payload();
        let path = |value: &Value| {
            value["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]
                ["startTimeUnixNano"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(path(&first), path(&second));
        let metrics = second["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap();
        assert_eq!(metrics.len(), 2);
        for metric in metrics {
            let points = metric["sum"]["dataPoints"].as_array().unwrap();
            assert_eq!(points.len(), COUNTER_COUNT);
            for point in points {
                assert!(point["asInt"].as_str().unwrap().parse::<i64>().is_ok());
                assert_eq!(point["attributes"].as_array().unwrap().len(), 2);
            }
        }
        assert!(second.to_string().len() < 64 * 1024);
        assert!(!second.to_string().contains("secret-path-and-owner"));
    }
}
