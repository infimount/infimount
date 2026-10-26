//! Shared, trusted control-plane contracts for the desktop, future CLI and
//! local IPC surfaces. This crate does not expose MCP tools or execute agents.
//! Publication authorization, mutation locks and byte verification remain in
//! the single trusted apply implementation; the data contract must be reused
//! by every future control surface.
pub mod publication;
pub mod publication_apply;
pub mod publication_approval;
pub mod publication_approval_ledger;
pub mod publication_context;
pub mod publication_destination;
pub mod publication_engine;
pub mod publication_evidence;
pub mod publication_plan;
pub mod review;
pub mod review_evidence;
