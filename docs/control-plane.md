# v0.8.2 B1: shared trusted control plane

Status: **B1 foundation — publication request contract migrated**. This is
not a declaration that the full headless service or CLI/API has shipped.

## Design rule

The desktop, future `infimountctl` and future local IPC API must use the
**same** prepare/review/preview/apply authority. A headless entry point is not
allowed to copy the desktop publication engine or bypass its locks.

The new workspace crate, `infimount_control`, is for trusted control-plane
contracts and orchestration. It is **not** an MCP server, agent adapter,
storage provider, or authorization token. Agents cannot call publication apply
through this crate.

## Implemented in this slice

- Moved `AgentTaskPublicationRequest`, the reviewed output type, and the
  fail/rename conflict-policy type from Tauri into
  `infimount_control::publication` with the same serialized JSON shape.
- Moved canonical reviewed-output selection validation: UUID request fields,
  bounded output count, path confinement, lowercase SHA-256, portable
  case-collision detection, deterministic output ordering, and destination
  directory normalization.
- Re-exported the shared types through the existing Tauri publication module,
  preserving the desktop UI command contract.
- Added tests for path escapes, duplicate portable paths, missing output
  selection, stable JSON field names and rejection of unknown authority fields.

## Trust and behavioral compatibility

The **same** Tauri handlers still own `preview_agent_task_publication` and
`publish_agent_task_outputs`. They still acquire the lifecycle/configuration/
workspace locks in their existing order, rebuild publication plans, compare
preview fingerprints, verify exact source/output hashes, enforce create-only
fail/rename semantics, and write verified unique receipts.

No model-visible publication tool, new external endpoint, permission
bypass, default client adapter or release tag is introduced. The existing
v0.8.1 104-argument Codex fixture and the A4 opt-in Claude pilot are unchanged.

## Next B1 slices

1. Extract shared reviewed-output enumeration and hash evidence without
   changing Tauri semantics; prove exact desktop parity with fixtures.
2. Share the destination plan/fingerprint and receipt contracts, keeping
   mutation locks and authorization authoritative.
3. Move the end-to-end trusted prepare/review/apply service into
   `infimount_control`. Tauri becomes a thin wrapper.
4. Only after parity tests, build the B2 CLI against the shared services.
   Headless publication must require explicit, server-controlled human
   approval or a narrowly scoped audited delegation; a `--yes` switch is
   not sufficient.

See [ADR 0008](adr/0008-trusted-headless-control-plane.md) and the
[v0.8.2 roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).
