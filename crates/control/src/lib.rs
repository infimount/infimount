//! Shared, trusted control-plane contracts for the desktop, future CLI and
//! local IPC surfaces. This crate does not expose MCP tools or execute agents.
//! Publication authorization, mutation locks and byte verification remain in
//! the single trusted apply implementation; the data contract must be reused
//! by every future control surface.
pub mod openai_hosted_artifacts;
pub mod openai_hosted_events;
pub mod openai_hosted_items;
// Private, offline-only crash-recovery and provider session-ID evidence.
#[allow(dead_code)]
mod hosted_execution_recovery;
pub mod openai_hosted_request;
pub mod publication;
pub mod publication_apply;
pub mod publication_approval;
pub mod publication_approval_ledger;
pub mod publication_context;
pub mod publication_destination;
pub mod publication_engine;
pub mod publication_evidence;
pub mod publication_plan;
pub mod publication_presence_challenge;
pub mod publication_presence_proof;
pub mod review;
pub mod review_evidence;
pub mod sandbox_bundle;
pub mod sandbox_import;
pub mod sandbox_memory_ingest;
pub mod sandbox_reviewed_ingest;
// Unix output import is internal until workspace locks and native review qualify.
#[cfg(unix)]
#[allow(dead_code)]
pub mod sandbox_local_export;
#[cfg(unix)]
#[allow(dead_code)]
mod sandbox_local_ingest;
