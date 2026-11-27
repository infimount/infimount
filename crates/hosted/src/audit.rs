//! Bounded structured audit records for collection by the deployment operator.
//! Never log raw requests, file contents, credentials, grants, or backend errors.
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::auth::Principal;
use crate::error::Result;

#[derive(Serialize)]
pub struct AuditEvent<'a> {
    pub event_id: String,
    pub timestamp_unix_seconds: u64,
    pub operation: &'a str,
    pub issuer: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub tenant: Option<&'a str>,
    pub client_id: Option<&'a str>,
    pub decision: &'static str,
    pub error_code: Option<&'static str>,
    pub path_sha256: Option<String>,
    pub bytes: u64,
}

impl<'a> AuditEvent<'a> {
    pub fn new(
        principal: Option<&'a Principal>,
        operation: &'static str,
        result: Result<()>,
        path: Option<&str>,
        bytes: u64,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp_unix_seconds: crate::now_epoch_seconds(),
            operation,
            issuer: principal.map(|principal| principal.issuer.as_str()),
            subject: principal.map(|principal| principal.subject.as_str()),
            tenant: principal.map(|principal| principal.tenant.as_str()),
            client_id: principal.map(|principal| principal.client_id.as_str()),
            decision: if result.is_ok() { "allowed" } else { "denied" },
            error_code: result.err().map(|error| error.code()),
            path_sha256: path.map(|path| format!("{:x}", Sha256::digest(path.as_bytes()))),
            bytes,
        }
    }
}

pub fn record(
    principal: Option<&Principal>,
    operation: &'static str,
    result: Result<()>,
    path: Option<&str>,
    bytes: u64,
) {
    crate::metrics::record(operation, result.is_ok(), bytes);
    let event = AuditEvent::new(principal, operation, result, path, bytes);
    // Fields are serialized as one JSON value inside the structured tracing
    // envelope. This preserves quoted identity strings without log injection.
    if let Ok(serialized) = serde_json::to_string(&event) {
        tracing::info!(target: "infimount_hosted::audit", event = %serialized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HostedError;

    #[test]
    fn rejected_input_is_not_copied_into_audit_records() {
        let secret_path = "/Reports/private-name?access_token=never-log-this";
        let event = AuditEvent::new(
            None,
            "http_request",
            Err(HostedError::Forbidden),
            Some(secret_path),
            0,
        );
        let value = serde_json::to_value(event).unwrap();
        let text = value.to_string();
        assert!(!text.contains("private-name"));
        assert!(!text.contains("never-log-this"));
        assert_eq!(value["decision"], "denied");
        assert_eq!(value["error_code"], "forbidden");
        assert_eq!(value["path_sha256"].as_str().unwrap().len(), 64);
        assert!(value["subject"].is_null());
    }
}
