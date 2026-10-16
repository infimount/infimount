# 0006 — Separate Agent Tasks from execution evidence

Date: 2026-10-16

## Status

Accepted for v0.8.2 implementation. No v0.8.1 storage or task schema migration is authorized by this decision.

## Context

The shipped v0.8.1 `AgentTaskManifest` contains the prepared task identity, title, prepared input paths and digests, and workspace binding. `codex_handoff.rs` independently launches a client. Client versions, runtime IDs, host paths, and task attempt status do not belong in the agent-readable manifest.

Adding execution details to the task makes reruns and other client adapters mutate an otherwise stable task contract. Storing execution evidence under the task-scoped `tasks/<uuid>/` tree lets the agent rewrite evidence. Storing it inside the workspace `.infimount/` directory is also insufficient by itself because a separately authorized general MCP client may have wider workspace access.

## Decision

1. Keep the existing v0.8.1 Agent Task manifest readable and unchanged. Prepared inputs and `TASK.md` remain the work contract. Do not add client/runtime/provider identity to it.
2. One task may have multiple execution attempts. Give each attempt a UUID `executionId` and a separate schema-versioned private record.
3. Store authoritative execution records in an application-controlled private state registry outside Agent Task and workspace trees. The implementation must also prevent any Infimount public MCP path from reading or writing this registry even when a registered Local Filesystem source overlaps its host parent. Filesystem permissions alone cannot isolate two processes running as the same OS user.
4. Scope the record to `workspaceId`, `taskId`, and `executionId`. Use bounded JSON, deny unknown fields, atomic create/transition, and explicit lifecycle state validation.
5. `prepared -> running -> succeeded | failed | cancelled` is the only valid terminal transition path. Reject unexpected transitions, duplicate IDs, and stale concurrent updates. Failed/cancelled executions must not be reported as success.
6. Persist client adapter kind/version, local executable digest when relevant, isolation mode, safe tool allowlist, task-scope digest, output digests/counts, safe error codes, and timestamps. Never persist credentials, raw prompts, environment variables, source contents, host paths, or provider secrets.
7. Execution evidence is operational integrity evidence, not a cryptographic attestation of an external provider or defense against a fully compromised local account. Explain its trust level accurately.
8. Keep `outputs/` under the existing task tree for v0.8.2 compatibility. A second execution must not silently reuse existing reviewable outputs. Preparing a distinct task remains the safe retry path until execution-scoped output namespaces are separately designed.

## Rejected alternatives

- Embed `client`, `model`, or `provider` in the task manifest: ties the immutable data contract to one execution.
- Keep evidence inside task `outputs/`: agent-writable and not authoritative.
- Assume `.infimount/` inside the workspace is private: broader workspace MCP exposure can invalidate that assumption.
- Overwrite a prior execution record: destroys evidence needed to explain a publication.

## Consequences and tests

- A2 implements an independently versioned execution registry with cross-process serialization and migration policy.
- Fixture schema version 1 is under `docs/schemas/agent-task-execution-v1.schema.json`.
- Reject unknown/sensitive fields, invalid transitions, paths, forbidden tools, oversized values, malformed digests, and inconsistent output state.
- Tests must show agent writes via task-scoped and general MCP cannot mutate authoritative execution evidence.
- Historical v0.8.1 manifests and publication receipts remain readable.
