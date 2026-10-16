# 0008 — Trusted headless control plane and publication authority

Date: 2026-10-16

## Status

Accepted for v0.8.2 implementation.

## Context

The v0.8.1 desktop can prepare, review, preview, and apply publication. A headless `infimountctl` and local IPC API will expose the same functionality to automation. A model that can invoke a shell or MCP client might also invoke a CLI if its commands are treated as unconditional authority. Operating-system-local transport alone does not distinguish a human from an agent running under the same account.

## Decision

1. Extract shared trusted task-control services into `infimount_control`. Tauri, CLI, and local IPC call the same authorization, lock, review, destination plan, and create-only implementation. No copy-pasted publication engine.
2. Keep storage administration outside public agent-facing MCP. A CLI is a trusted control-plane entry point, not a new model tool.
3. Initial IPC uses Unix domain sockets or Windows named pipes with restrictive local ACLs and peer/process checks where supported. No TCP listener in the initial release. OS authentication is necessary but not sufficient for a sensitive publish.
4. Separate preparing/reviewing/previewing from **authorization to apply**. The final apply requires a server-side record of a human approval or a separately configured, narrowly scoped delegation. Do not infer human approval from CLI invocation, possession of a preview hash, a boolean `--yes`, or a model-returned message.
5. Human approval binds task ID, output hashes, destination namespace/revision, destination paths, conflict mode, preview fingerprint, actor, expiration, and a single-use nonce. Apply verifies the grant and recomputes all bytes and destination state under existing mutation locks.
6. Initial v0.8.2 headless mode supports preview and dry-run without additional consent. Publication may require local desktop mediation. Unattended automatic publishing is disabled until an explicit delegation design supports bounded scope, expiry, revocation, and audit.
7. Store delegation/approval state outside agent-reachable paths and invalidate it after consumption, expiration, changes to reviewed output, destination or policy, or relevant configuration revisions.
8. Return schema-versioned privacy-safe JSON with stable error codes. Do not emit secrets, raw source paths, original prompts, or full configuration to automation outputs.
9. One control-plane process owns state transitions and journal recovery, using the existing configuration/workspace lock acquisition order (ADR 0005). No silent bypass when the desktop process is absent.

## Rejected alternatives

- `infimountctl publish --yes` as a sufficient authorization boundary.
- An unauthenticated local HTTP control server.
- Give the task-scoped MCP server a final publication tool.
- Trust file mode/ownership alone to distinguish an agent from its owner.

## Consequences and tests

B1-B4 must prove desktop/CLI/IPC parity. Hostile agent with terminal access must not be able to publish without a trusted approval. Preview replay, stale output, grant replay, destination drift, revoked grants, and attempted use of another task's grant must fail closed. Agent Tasks remain usable without enabling a headless control server.
