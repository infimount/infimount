//! Fixed-cardinality, process-lifetime counters derived from audit events.
//! Identity, paths, tokens, transfer IDs and arbitrary labels are never accepted.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;

const OPERATIONS: [&str; 20] = [
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
static REPORTER_STARTED: AtomicBool = AtomicBool::new(false);

struct Counters {
    events: [AtomicU64; COUNTER_COUNT],
    bytes: [AtomicU64; COUNTER_COUNT],
}

#[derive(Serialize)]
struct Snapshot {
    event: &'static str,
    timestamp_unix_seconds: u64,
    counters: Vec<CounterSnapshot>,
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
            events: [const { AtomicU64::new(0) }; COUNTER_COUNT],
            bytes: [const { AtomicU64::new(0) }; COUNTER_COUNT],
        }
    }

    fn record(&self, operation: &str, allowed: bool, bytes: u64) {
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
        }
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

/// Start one reporter after the hosted listener has successfully started.
/// Constructing a router or checking configuration does not start reporting.
pub fn start_reporting() {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if REPORTER_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    runtime.spawn(async {
        let mut interval =
            tokio::time::interval_at(tokio::time::Instant::now() + REPORT_PERIOD, REPORT_PERIOD);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Ok(snapshot) = serde_json::to_string(&COUNTERS.snapshot()) {
                tracing::info!(target: "infimount_hosted::metrics", snapshot = %snapshot);
            }
        }
    });
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
}
