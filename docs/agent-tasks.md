# Agent Tasks

Agent Tasks are the v0.8.1 workflow for bringing selected storage files into a scoped local Agent Workspace, letting an existing MCP client work on those prepared copies, reviewing the resulting files in Infimount, and publishing only outputs the desktop user explicitly approves.

The first release intentionally composes existing Infimount storage, workspace, MCP policy, client-integration, transfer, confirmation, and audit primitives. It does not introduce an agent runtime, a semantic index, a synchronization engine, or a second authorization system.

## Product contract

The recurring workflow is:

```text
source storage
  -> prepare selected files into a local task workspace
  -> existing agent works through the workspace's MCP boundary
  -> outputs are discovered under outputs/
  -> user reviews and selects outputs in the desktop app
  -> selected unchanged bytes are published through Infimount
```

Task preparation never mutates the selected source. Creating a task also never exposes the source storage to MCP. Existing unrelated MCP exposure remains authoritative and must be disclosed by the UI rather than hidden behind an Agent Task guarantee.

## Task package

A committed task is ordinary inspectable files under an existing Agent Workspace:

```text
<workspace-root>/
  tasks/
    <task-uuid>/
      TASK.md
      task-manifest.json
      inputs/
      outputs/
      publish-receipt.json   # present only after publication is attempted
```

`TASK.md` is human- and agent-readable guidance. `task-manifest.json` describes the prepared input snapshot. Neither file is an authorization source; the existing workspace/MCP policy remains authoritative even if an agent edits task files.

Task roots in manifest schema v1 are exactly one direct child of `tasks/` and are derived from the task UUID. Nested forms such as `tasks/a/b` are rejected so later task discovery has one unambiguous layout.

### Manifest v1

The agent-readable manifest records:

- schema version;
- task UUID;
- display title;
- creation timestamp;
- containing Agent Workspace UUID;
- workspace-relative task root;
- the fixed `outputs` directory;
- prepared input records containing only task-relative input path, byte size, and lowercase SHA-256 of the copied bytes.

The original storage ID and remote source path are deliberately not persisted in the agent-readable package. They are needed while preparing the copy, but exposing the full bucket/drive/filesystem hierarchy to the agent provides no authority and may reveal unrelated namespace information. The prepared bytes and their task-relative paths are the task's input snapshot.

The manifest contains no credentials, OAuth material, policy grants, confirmation state, or publication authority. Unknown fields are rejected by the typed reader so a future producer cannot smuggle new authority semantics into a v1 manifest.

## Preparation

The first v0.8.1 preparation implementation accepts one source storage and an existing **read-write Agent Workspace backed by Local Filesystem storage**. The source may be any enabled OpenDAL-backed storage with stat/read/list capability.

Preparation performs these steps server-side:

1. Validate the task brief and selected source paths.
2. Re-verify the Agent Workspace schema and storage namespace fingerprint.
3. Plan the selection using the existing transfer planner with copy + fail-on-conflict semantics.
4. Reject empty selections, overlapping selections, case-insensitive destination collisions, more than 10,000 prepared files, or more than 2 GiB of prepared bytes.
5. Create a hidden `tasks/.preparing-<uuid>` package inside the workspace.
6. Execute exactly the validated transfer-plan entries into `inputs/` with bounded streaming copies. The source tree is not recursively rediscovered after the authoritative file-count/byte-limit checks, so entries added after planning cannot enter the task package or bypass those limits. The source is never moved.
7. Re-stat planned source entries while executing and fail closed on removal, file/directory type drift, or planned file-size drift.
8. Stream SHA-256 over each prepared destination file instead of loading whole files into memory, and verify the prepared file count and byte total still match the validated plan.
9. Write the validated manifest and `TASK.md`.
10. Re-check local symlink/reparse confinement and commit the package with a local OpenDAL directory rename to `tasks/<uuid>`.
11. On failure, delete only the hidden staging tree created for that task. If cleanup itself fails, surface a cleanup-required error rather than reporting a clean failure.

Preflight performs the same bounded planning without mutating storage and reports selected item count, expanded file/directory count, total bytes, and MCP-exposure warnings. Preflight is a scope/size check, not authorization of an immutable byte snapshot. Prepare always plans again server-side and the committed manifest describes the bytes actually prepared.

### Concurrency and local-path boundary

Workspace create/update/delete is serialized against the complete preparation transaction with the existing cross-process workspace mutation lock. Storage namespace-changing edits are already rejected while workspaces are bound.

For Local Filesystem storage, preparation reuses the v0.8 symlink/Windows reparse-point path guard before relevant source/destination operations and again immediately before package commit. This retains the already documented check-then-OpenDAL race limitation; eliminating that class of race requires a future handle-relative/openat2-style local backend rather than a separate Agent Task path implementation.

The hidden staging folder is not an authorization mechanism. If a workspace already has independent MCP exposure, that existing policy remains authoritative throughout preparation and is disclosed to the user. Task metadata never expands or narrows that policy by itself.

## Initial limits

The v0.8.1 implementation starts with conservative limits:

- at most 1,000 explicitly selected source items per preparation request;
- at most 10,000 expanded prepared input files per task;
- at most 2 GiB of prepared input bytes per task;
- at most 100 explicitly requested output paths in `TASK.md`;
- task package paths use canonical relative forward-slash form;
- prepared input paths must be children of `inputs/`;
- requested output paths must be children of `outputs/`;
- prepared/requested paths that collide case-insensitively are rejected for cross-platform portability;
- task roots must be exactly `tasks/<task-uuid>`.

These are product safety/usability ceilings, not targets for bulk ingestion. Agent Tasks are not a replacement for a data-ingestion or synchronization pipeline.

## Security invariants

The v0.8.1 implementation must preserve these invariants:

1. Preparing a task never mutates the source selection.
2. Preparing a task never grants MCP access to the source storage.
3. Existing storage/workspace policy remains the authorization source.
4. Agent Task publication is a desktop action, not an MCP administration tool.
5. Only outputs explicitly selected by the user may be published.
6. The SHA-256 reviewed by the user must still match immediately before publication; stale output requires review again and writes nothing for that file.
7. Initial publication is new-file-only. Existing destination collisions fail closed rather than silently overwriting.
8. Publication results are reported per file; partial success is never presented as complete success.
9. Manifest, task brief, receipts, product events, and diagnostics never contain storage credentials or OAuth secrets.
10. All storage I/O continues to use the existing OpenDAL-backed abstraction.
11. Existing local symlink/reparse-point defenses remain in force.
12. Task metadata never expands access beyond the policy already enforced by Infimount.

## Deliberate non-goals for v0.8.1

The first Agent Tasks release does not include:

- a built-in AI chat or model runtime;
- direct Agent Task publication tools over MCP;
- continuous or bidirectional sync;
- arbitrary overwrite/delete publication;
- whole-storage snapshots or universal rollback;
- vector/semantic indexing;
- remote multi-user task collaboration;
- a provider-specific storage implementation outside OpenDAL;
- an Infimount-hosted backend.

If implementing the workflow requires one of those foundational systems, that part should be redesigned explicitly instead of being hidden inside the task feature.

## Planned delivery slices

1. **Task package foundation** — typed manifest/brief validation and `TASK.md` rendering in `infimount-core`.
2. **Prepare task backend** — server-side preflight, exact-plan local-workspace copy, destination hashing, and manifest/brief creation.
3. **Prepare from File Browser** — selected-item UX, workspace choice, scope review, and prepare result presentation using the backend contract from slice 2.
4. **Agent handoff** — reuse existing client adapter preview/apply and workspace policy; disclose existing independent source exposure; add Codex as a first-class client.
5. **Output review** — discover `outputs/`, reuse previews, hash output bytes, select approved files.
6. **Safe publication** — stale-hash recheck, new-file-only destination writes, collision handling, per-file receipt.
7. **Pilot evidence** — privacy-bounded task lifecycle product events, sample tasks, real screenshots, rc validation and normal v0.8.0 -> v0.8.1 updater smoke.

## Integration direction

Agent Tasks are designed to work with existing MCP clients. The desktop remains the storage control plane; client-specific packaging should reuse the same MCP server and policy rather than implementing separate storage access paths. Codex is the first planned plugin/skill packaging target, with Pi as a lightweight follow-on and OpenCode continuing to use its existing direct MCP integration.
