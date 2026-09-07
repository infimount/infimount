# Agent Tasks

Agent Tasks are the v0.8.1 workflow for bringing selected storage files into a scoped local Agent Workspace, letting an existing MCP client work on those prepared copies, reviewing the resulting files in Infimount, and publishing only outputs the desktop user explicitly approves.

The first release intentionally composes existing Infimount storage, workspace, MCP policy, client-integration, transfer, confirmation, and audit primitives. It does not introduce an agent runtime, a semantic index, a synchronization engine, or a second authorization system.

## Product contract

The recurring workflow is:

```text
source storage
  -> prepare selected files into a local task workspace
  -> existing agent works only through the workspace's MCP boundary
  -> outputs are discovered under outputs/
  -> user reviews and selects outputs in the desktop app
  -> selected unchanged bytes are published through Infimount
```

Task preparation never mutates the selected source. Creating a task also never exposes the source storage to MCP. Existing unrelated MCP exposure remains authoritative and must be disclosed by the UI rather than hidden behind an Agent Task guarantee.

## Task package

A task is ordinary inspectable files under an existing Agent Workspace:

```text
<workspace-root>/
  tasks/
    <task-name-or-id>/
      TASK.md
      task-manifest.json
      inputs/
      outputs/
      publish-receipt.json   # present only after publication is attempted
```

`TASK.md` is human- and agent-readable guidance. `task-manifest.json` is provenance for the prepared input snapshot. Neither file is an authorization source; the existing workspace/MCP policy remains authoritative even if an agent edits task files.

### Manifest v1

The core manifest records:

- schema version;
- task UUID;
- display title;
- creation timestamp;
- containing Agent Workspace UUID;
- workspace-relative task root;
- the fixed `outputs` directory;
- prepared input records containing source storage ID/path for provenance, task-relative input path, byte size, and lowercase SHA-256 of the copied bytes.

The manifest contains no credentials, OAuth material, policy grants, confirmation state, or publication authority. Unknown fields are rejected by the typed reader so a future producer cannot smuggle new authority semantics into a v1 manifest.

## Initial limits

The v0.8.1 implementation starts with conservative limits:

- at most 10,000 prepared input files per task;
- at most 100 explicitly requested output paths in `TASK.md`;
- task package paths use canonical relative forward-slash form;
- prepared input paths must be children of `inputs/`;
- requested output paths must be children of `outputs/`;
- task roots must be children of `tasks/`.

A later preparation preflight may impose smaller interactive byte/file limits for usability. The 10,000-input format ceiling is not permission to ingest an arbitrarily large bucket.

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
2. **Prepare from File Browser** — selection preflight, local workspace choice, source copy into `inputs/`, manifest and brief creation.
3. **Agent handoff** — reuse existing client adapter preview/apply and workspace policy; disclose existing independent source exposure.
4. **Output review** — discover `outputs/`, reuse previews, hash output bytes, select approved files.
5. **Safe publication** — stale-hash recheck, new-file-only destination writes, collision handling, per-file receipt.
6. **Pilot evidence** — privacy-bounded task lifecycle product events, sample tasks, real screenshots, rc validation and normal v0.8.0 -> v0.8.1 updater smoke.

## Integration direction

Agent Tasks are designed to work with existing MCP clients. The desktop remains the storage control plane; client-specific packaging should reuse the same MCP server and policy rather than implementing separate storage access paths.
