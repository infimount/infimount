//! Explicitly enabled, stateless, identity-scoped artifact data plane.
//! The desktop sidecar and its default loopback transport are independent.
pub mod audit;
pub mod auth;
pub mod config;
pub mod error;
pub mod mcp;
pub mod metrics;
pub mod storage;
pub mod transfers;
pub mod transport;

pub fn now_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
