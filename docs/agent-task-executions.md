# Agent Task execution evidence (v0.8.2 A2)

Status: Implemented as a private Rust store. No new client adapter or
agent-visible tool is enabled by this milestone.

## Why the store is separate

The existing v0.8.1 Agent Task manifest remains immutable and is still
agent-readable inside the task package. An execution is one attempt to run that
same prepared task with a particular client/runtime. The execution record is
neither task input nor approval to publish generated outputs.

The v1 record schema is in
[`docs/schemas/agent-task-execution-v1.schema.json`](schemas/agent-task-execution-v1.schema.json).
The production validator is implemented in `crates/core/src/agent_task_executions.rs`.

## State and storage

```text
<Infimount private config directory>/
  agent-task-executions/
    .mutations.lock
    <workspace-uuid>/
      <task-uuid>/
        <execution-uuid>.json
```

The store uses canonical lowercase UUID file components and requires every one
of the 21 version-1 fields. Unknown/duplicate/omitted fields, invalid client
or provider combinations, invalid times, malformed hashes, unsafe error codes,
and invalid tool names fail closed.

```text
prepared -> running -> succeeded
        |        \----> failed
        |        \----> cancelled
        \--------------> failed
        \--------------> cancelled
```

Creation now also holds a cross-process **single-active-execution gate per task**:
under the same mutation lock, it validates all previous records for that task
and rejects any new attempt while another execution remains in `prepared` or
`running` state. This applies even when the new attempt has a different UUID.
Malformed, unexpected, or interrupted historical evidence blocks a new attempt
rather than allowing an ambiguous concurrent run. Terminal attempts can be
followed by a new execution only when the caller separately proves there are
no existing reviewable outputs.

A stranded active record after a crash deliberately blocks new execution
until a reviewed recovery flow has confirmed the owned process has terminated.
Do not silently expire a lease: the old agent might still be writing outputs.

Each transition compares the caller's complete previously-read record with the
current record while holding a cross-process lock. Terminal states have no
outgoing transitions. A different instance cannot silently advance a stale
revision. Failed/cancelled outcomes carry safe error codes, never raw logs.

Creation exposes completed JSON via no-replace hard linking from a flushed
temporary file. Transitions use atomic file replacement; user-visible
publication receipts and existing v0.8.1 task packages are **not rewritten**.

## MCP confidentiality and limits

The authoritative execution registry is **not inside an Agent Workspace**.
The Local Filesystem MCP path gate denies the private execution subtree even
when an explicitly exposed broad Local Filesystem storage overlaps it, and
prevents write-like operations against its ancestor directories (including
move/delete of the Infimount config directory). Directory traversal and
symlink/reparse-point guards continue to apply.

General MCP directory listing/search filters denied descendants. The task
scoped MCP server is independently restricted to its prepared task prefix.
No MCP resource exposes the execution store directly. Custom config-directory
deployments must use the same private-state path guard before enabling general
MCP access; the default desktop/sidecar uses the shared Infimount config root.

The private store is *operational integrity evidence*. It does not cryptographically
attest remote provider behavior and does not protect against a malicious
process already executing with unrestricted filesystem access as the owner.

## Compatibility and execution plumbing

A2 provisions an `ExecutionRecordStore` on desktop `AppState` and
provides `create`, `read`, and `transition` services for later phases.
It does **not** automatically create execution records when Codex launches.
A3 will attach lifecycle recording to the adapter registry after defining the
process/completion ownership contract.

The v0.8.1 Codex command, its 104-argument golden fixture, source
snapshot hashes, MCP tool allowlist, output review, and create-only publication
remain unchanged.

## Verification

- Rust unit tests validate all A0 accepted and rejected JSON fixtures.
- Unknown/missing/duplicate fields and secret/path fields are rejected.
- Duplicate execution IDs cannot overwrite existing evidence.
- Two concurrent attempts with different IDs for the same task yield exactly one
  successful reservation; the other is rejected even across process instances.
- Corrupted prior records and interrupted temporary records prevent an
  ambiguous new execution.
- Concurrent writers and stale records fail rather than silently changing state.
- Execution identity, scope digest, and enabled tools remain immutable.
- Terminal states cannot reopen.
- Private Unix file and directory permissions are checked.
- Private symlinks are not followed.
- Broad Local Filesystem MCP roots cannot read/write/list execution records,
  nor rename/delete a parent containing the registry.
- All normal CI, audit, packaged release, MSRV and Windows checks remain gates.

No new release tag is created by this milestone.
