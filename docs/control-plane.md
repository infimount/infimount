# v0.8.2 B1: shared trusted control plane

Status: **B1 foundation — publication, review, evidence, fingerprint and
receipt wire contracts extracted**. The full headless service and CLI/API
have not shipped.

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

## B1.2: shared read-only reviewed-output evidence

`infimount_control::publication_evidence::verify_reviewed_output_digest`
now performs the same existence, file type, byte-size, and exact SHA-256
comparison formerly in the Tauri publication planner. Desktop calls the shared
verifier after it constructs a task-relative source path and applies the
existing storage namespace/local confinement check. Rust memory-operator tests
cover exact bytes, missing paths, size changes, same-size changes, directories,
invalid output paths, and malformed SHA-256.

This helper **does not** enumerate arbitrary storage, authorize publication,
lock remote writers, or apply a write. A caller must bind the workspace, task,
storage identity and source path before using it. The authoritative publish
path still re-verifies bytes under the original mutation lock sequence.

## B1.3: shared publication plan, fingerprint and receipt contracts

`infimount_control::publication_plan` now owns plan-file/action wire types,
the staleness fingerprint shape and its exact JSON-byte SHA-256 calculation,
and the publication receipt payload shape. The desktop still owns actual
destination discovery, collision handling, locking, create-only writes,
destination-byte verification and receipt persistence.

A pinned test retains the existing preview-token digest for a known fixture,
and receipt JSON tests preserve field names while excluding source storage
identity and host paths. This is **wire compatibility, not authorization**.
A valid preview token on its own never approves publication.

## B1.4: shared read-only review wire contract

The `infimount_control::review` module contains the unchanged desktop
review request, review file and review response schemas. Tauri re-exports the
types so the command and frontend JSON remain compatible. Tests pin the legacy
camelCase shape, explicit preview-unavailable reasons and the absence of any
publication authority fields. Unknown review request fields are rejected.

The desktop still owns workspace binding, manifest checks, local path
confinement, bounded recursive output enumeration, exact SHA-256 hashing
and preview re-verification. This slice introduces no new review endpoint,
agent tool, approval or publication behavior.

## B1.5: shared fail-closed output enumeration and byte caps

`infimount_control::review_evidence` lists task output files using the
existing OpenDAL recursive traversal. It rejects potentially truncated
10,000-entry listings, limits review to 1,000 files and 2 GiB of listed
bytes, sorts paths deterministically, and rechecks the 2 GiB cap against
actual hashed bytes. Tests cover nested listings, directory filtering,
truncation, file-count ceilings, size ceilings and integer overflow.

The desktop still validates its bound task manifest, storage namespace,
output directory, and every returned local path before hashing or previewing
each file. The same 20-preview and 64 KiB per-preview limits remain in the
desktop implementation. The helper is read-only evidence collection, not
an approval, a publication token, or an agent-facing capability.

## B1.6: shared read-only destination choice and collision planning

`infimount_control::publication_destination` now owns the same OpenDAL
existence checks, create/conflict/rename choice, bounded legacy " copy" suffix
search and case-insensitive collision reservations used by the desktop planner.
Memory-operator tests cover existing and missing objects, fail versus rename
policies, exact suffixes, dotfiles, path-length failures and non-mutating
read-only planning.

**Not publication authority:** the desktop still resolves task/workspace
identity, checks backend capabilities, validates storage namespaces and
destination confinement, rejects reserved task paths, rehashes reviewed bytes,
and replans under lifecycle/configuration/workspace locks immediately before
create-only writes and receipt persistence. This does not create an MCP tool,
headless apply path or new UI.

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

1. Move the destination context and configuration/workspace guards into the
   single trusted control service, without changing lock ordering.
2. Move the end-to-end trusted prepare/review/apply service into
   `infimount_control`. Tauri becomes a thin wrapper.
3. Only after parity tests, build the B2 CLI against the shared services.
   Headless publication must require explicit, server-controlled human
   approval or a narrowly scoped audited delegation; a `--yes` switch is
   not sufficient.

See [ADR 0008](adr/0008-trusted-headless-control-plane.md) and the
[v0.8.2 roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).
