# Agent Tasks

Agent Tasks are the next Infimount workflow for bounded agent work on existing files. The complete implementation is on `main` and is targeted for **v0.9.0**. The current v0.8.0 stable release does not include this workflow.

The design composes existing Infimount storage, Agent Workspace, MCP policy, client-integration, transfer, confirmation, and audit primitives. It does not introduce an agent runtime, semantic index, synchronization engine, hosted backend, or second authorization system.

## Product contract

The recurring workflow is:

```text
source storage
  -> prepare selected files into a scoped local task workspace
  -> existing agent works through the task/workspace MCP boundary
  -> outputs are discovered under outputs/
  -> user reviews output bytes in Infimount
  -> user explicitly selects outputs to publish
  -> Infimount previews the exact destination plan
  -> unchanged selected bytes are published with create-only writes
```

Task preparation never mutates the selected source. Creating a task also never grants MCP access to the source storage. Existing unrelated MCP exposure remains authoritative and is disclosed by the desktop UI rather than hidden behind an Agent Task guarantee.

## Task package

A committed task is ordinary, inspectable files under an existing Agent Workspace:

```text
<workspace-root>/
  tasks/
    <task-uuid>/
      TASK.md
      task-manifest.json
      inputs/
      outputs/
      publish-receipt-<publication-id>.json  # zero or more successful publications
```

`TASK.md` is human- and agent-readable guidance. `task-manifest.json` describes the prepared input snapshot. Neither file is an authorization source. Existing workspace and MCP policy remain authoritative even if an agent edits task files.

Task roots in manifest schema v1 are exactly one direct child of `tasks/` and are derived from the task UUID. Nested forms such as `tasks/a/b` are rejected so task discovery has one unambiguous layout.

### Manifest v1

The manifest records:

- schema version;
- task UUID;
- display title;
- creation timestamp;
- containing Agent Workspace UUID;
- workspace-relative task root;
- the fixed `outputs` directory;
- prepared input records containing only task-relative input path, byte size, and lowercase SHA-256 of the copied bytes.

The original storage ID and remote source path are deliberately not persisted in the agent-readable package. They are needed while preparing the copy, but exposing the full bucket, drive, or filesystem hierarchy provides no task authority and may reveal unrelated namespace information. The prepared bytes and their task-relative paths are the task's input snapshot.

The manifest contains no credentials, OAuth material, policy grants, confirmation state, or publication authority. Unknown fields are rejected by the typed reader so a future producer cannot smuggle new authority semantics into a v1 manifest.

## Preparation

The first Agent Tasks implementation accepts one source storage and an existing **read-write Agent Workspace backed by Local Filesystem storage**. Agent Workspace creation remains read-only by default. The desktop user must explicitly enable agent writes before creating a workspace that will host Agent Task outputs.

The source may be any enabled OpenDAL-backed storage with the required stat, read, and list capabilities.

Preparation performs these steps server-side:

1. Validate the task brief and selected source paths.
2. Re-verify the Agent Workspace schema and storage namespace fingerprint.
3. Plan the selection using the existing transfer planner with copy plus fail-on-conflict semantics.
4. Reject empty selections, overlapping selections, case-insensitive destination collisions, more than 10,000 prepared files, or more than 2 GiB of prepared bytes.
5. Create a hidden `tasks/.preparing-<uuid>` package inside the workspace.
6. Execute exactly the validated transfer-plan entries into `inputs/` with bounded streaming copies. The source tree is not recursively rediscovered after the authoritative file-count and byte-limit checks, so entries added after planning cannot enter the task package or bypass those limits. The source is never moved.
7. Re-stat planned source entries while executing and fail closed on removal, file/directory type drift, or planned file-size drift.
8. Stream SHA-256 over each prepared destination file and verify the prepared file count and byte total still match the validated plan.
9. Write the validated manifest and `TASK.md`.
10. Re-check local symlink/reparse confinement and commit the package with a local OpenDAL directory rename to `tasks/<uuid>`.
11. On failure, delete only the hidden staging tree created for that task. If cleanup itself fails, surface a cleanup-required error rather than reporting a clean failure.

Preflight performs the same bounded planning without mutating storage and reports selected item count, expanded file/directory count, total bytes, and MCP-exposure warnings. Preflight is a scope and size check, not authorization of an immutable byte snapshot. Prepare always plans again server-side, and the committed manifest describes the bytes actually prepared.

## Agent handoff and output review

The desktop can launch a prepared task in Codex using the existing Infimount MCP integration. Agent Tasks do not add a second storage-access path for Codex.

After agent work, Infimount discovers files under the task's `outputs/` directory and reviews them as a fresh server-side snapshot. Each review entry carries the task-relative path, byte size, SHA-256, and a bounded text preview when the file is previewable. Publication starts with **nothing selected**. Output review by itself publishes nothing.

A review is not a reusable write capability. Publication independently validates the requested output path, expected size, expected SHA-256, current task identity, destination, and current storage state.

## Safe publication

Publication is a separate desktop-only action. It is not exposed through Agent Task MCP and cannot be triggered automatically by the agent.

The publication flow is:

1. The user explicitly selects one or more freshly reviewed output files.
2. The user explicitly chooses destination storage, destination folder, and conflict policy.
3. The desktop requests a publication preview.
4. The backend rebuilds the exact destination plan from current output bytes and current destination state and returns a SHA-256 preview token.
5. Any output selection, destination, or conflict-policy edit invalidates the approved preview.
6. Apply takes the reviewed output descriptors plus preview token, locks the relevant configuration/workspace mutation boundary, rebuilds the complete plan again, and rejects token drift before writing.
7. Source output bytes are re-hashed immediately before opening each destination writer and again while streaming.
8. Destination writes use OpenDAL create-only semantics. A backend that cannot guarantee create-if-absent fails closed for Agent Task publication.
9. The committed destination is re-hashed after close and must match the reviewed digest.
10. A fully successful publication writes a unique create-only `publish-receipt-<publication-id>.json` inside the task package and verifies the committed receipt bytes.

Supported conflict policies are:

- **fail**: any existing destination blocks publication during preview;
- **rename**: Infimount chooses a non-conflicting destination name and shows that exact path in preview.

There is deliberately **no overwrite mode** for Agent Task publication.

Nested paths beneath `outputs/` are preserved beneath the chosen destination folder. When publishing into the task workspace's own storage namespace, `tasks/**` and `.infimount/**` are reserved case-insensitively and cannot be publication destinations. Destinations that overlap the reviewed source namespace are rejected.

### Partial failure semantics

Infimount never claims an all-or-nothing transaction when the destination backend cannot provide one. If a later file fails after earlier create-only writes have committed, the operation returns a cleanup-required error and preserves those committed objects rather than attempting a potentially unsafe delete rollback. The user can inspect the destination state before deciding what to remove.

A successful publication result lists the exact published task paths, destination paths, sizes, hashes, publication ID, timestamp, and receipt path. Receipts never contain original source-storage paths or credentials.

## Concurrency and local-path boundary

Workspace create, update, delete, task preparation, and publication serialize through the existing cross-process workspace/configuration mutation boundaries where required. Storage namespace-changing edits are already rejected while workspaces are bound.

For Local Filesystem storage, Agent Tasks reuse the v0.8 symlink and Windows reparse-point path guard before relevant operations. This retains the documented check-then-OpenDAL race limitation. Eliminating hostile local filesystem TOCTOU races requires a future handle-relative or `openat2`-style local backend rather than a separate Agent Task path implementation.

The task package is not an authorization mechanism. If a workspace or source storage already has independent MCP exposure, that policy remains authoritative throughout the workflow and is disclosed to the user.

## Initial limits

The first implementation starts with conservative limits:

- at most 1,000 explicitly selected source items per preparation request;
- at most 10,000 expanded prepared input files per task;
- at most 2 GiB of prepared input bytes per task;
- at most 100 explicitly requested output paths in `TASK.md`;
- at most 100 explicitly selected outputs per publication request;
- at most 2 GiB of selected output bytes per publication request;
- task package paths use canonical relative forward-slash form;
- prepared input paths must be children of `inputs/`;
- requested and reviewed output paths must be children of `outputs/`;
- prepared/requested paths that collide case-insensitively are rejected for cross-platform portability;
- task roots must be exactly `tasks/<task-uuid>`.

These are product safety and usability ceilings, not targets for bulk ingestion. Agent Tasks are not a replacement for a data-ingestion or synchronization pipeline.

## Security invariants

The implementation preserves these invariants:

1. Preparing a task never mutates the source selection.
2. Preparing a task never grants MCP access to the source storage.
3. Existing storage and workspace policy remain the authorization source.
4. Agent Task publication is a desktop action, not an MCP administration tool.
5. Nothing is selected for publication by default.
6. Only outputs explicitly selected by the user may be published.
7. The SHA-256 and byte size reviewed by the user must still match before and during publication.
8. Publication uses create-only writes with explicit **fail** or **rename** conflict handling and never overwrites an existing destination.
9. Destination bytes are verified after commit.
10. Partial multi-file failure is surfaced as cleanup-required when committed outputs may remain. It is never presented as complete success.
11. Successful publication receipts are unique and create-only.
12. Manifest, task brief, receipts, product events, and diagnostics never contain storage credentials or OAuth secrets.
13. All storage I/O continues to use the existing OpenDAL-backed abstraction.
14. Existing local symlink/reparse-point defenses remain in force.
15. Task metadata never expands access beyond the policy already enforced by Infimount.

## Deliberate non-goals for v0.9.0

The first Agent Tasks release does not include:

- a built-in AI chat or model runtime;
- direct Agent Task publication tools over MCP;
- continuous or bidirectional sync;
- arbitrary overwrite or delete publication;
- whole-storage snapshots or universal rollback;
- vector or semantic indexing;
- remote multi-user task collaboration;
- a provider-specific storage implementation outside OpenDAL;
- an Infimount-hosted backend.

If implementing the workflow requires one of those foundational systems, that part should be designed explicitly instead of being hidden inside the task feature.

## Delivery state

The product loop is implemented on `main` through six completed slices:

1. **Task package foundation**: typed manifest/brief validation and `TASK.md` rendering.
2. **Prepare task backend**: bounded preflight, exact-plan local-workspace copy, hashing, and package commit.
3. **Prepare from File Browser**: selected-item UX, workspace choice, scope review, and result presentation.
4. **Agent handoff**: existing client integration plus first-class Codex launch for the prepared task.
5. **Output review**: bounded discovery, preview, byte size, and SHA-256 review of `outputs/`.
6. **Safe publication**: mandatory preview, staleness token, create-only writes, fail/rename conflict handling, destination verification, and unique receipts.

The remaining v0.9.0 gate is **pilot evidence**, not more feature breadth. The pilot must exercise real coding, document, and data-analysis tasks, capture action-driven UI evidence, and verify the normal v0.8.0 to v0.9.0 release/update path before stable publication.

## Integration direction

Agent Tasks work with existing MCP clients. The desktop remains the storage control plane, and client-specific handoff reuses the same MCP server and policy rather than implementing separate storage access paths. Codex is the first first-class task handoff. Pi is a lightweight follow-on candidate, while OpenCode can continue using its existing direct MCP integration until pilot evidence justifies additional packaging.
