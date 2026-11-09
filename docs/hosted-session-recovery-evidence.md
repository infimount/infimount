# Infimount v0.8.2 C2.9a: private hosted session recovery evidence

Status: **internal, offline-only**. This code cannot call OpenAI, read API keys,
transfer prepared inputs, import outputs or approve original storage
publication. It deliberately has **no** desktop/Tauri/MCP/CLI command.

## Failure model

A hosted session-create POST can succeed even when the HTTP response or
event stream disconnects. Retrying blindly can create a second billed
session, duplicate work and orphaned provider-side data.

The new `hosted_execution_recovery` layer reuses the existing
`ExecutionRecordStore` and its private, size-bounded, create-only,
cross-process-locked records outside Agent Workspaces.

1. **Prepared.** Validate the trusted task manifest and separately reviewed
   redacted export brief through `build_sandbox_task_bundle`; require explicit
   output names and a strictly allowlisted model identifier. Persist the
   declared task-scope digest and separately domain-separated prompt-scope
   digest. This is **metadata only** and *not* authorization or fresh
   rehashing of task input bytes.
2. **Running before network.** Under a single cross-process compare-and-set
   transition, persist `Prepared -> Running` **before** a future paid
   session-create call. A restart that sees `Running` with no provider ID
   must stop and request account/session reconciliation, **never retry**
   session creation.
3. **First session ID.** When the trusted event parser receives a session
   ID, save it immediately as the **single allowed Running -> Running
   mutation**. The transition only adds a previously absent provider ID to
   a remote-sandbox running record. It rejects rebinds, stale updates,
   changed scope, changed tools, changed startup time and terminal rewrites.
4. **Recover.** Reopen the private store after restart to choose between
   fresh approval needed, manual cleanup if the session ID was lost,
   inspection of exactly the persisted session, and terminal/no-repeat.
   No agent-provided or browser-supplied completion flag is authoritative.

## Security limitations

The module remains private within `infimount_control`. No production code
invokes a charged POST from it. A future native coordinator must additionally
hold configuration and workspace mutation locks; verify user presence and
one-use export/spending consent; use the existing Unix no-follow actual-byte
export (C1.4), OS keyring credentials (C2.8), authenticated session
transport, bounded saved-item/artifact verification (C2.2-C2.4), deletion
and private cleanup evidence. Do not mark success before durable outputs
are verified and imported, or mark a still-live session terminal solely to
unblock a retry.

The one-time provider-ID binding preserves the execution record's 21-field
schema and does not add credentials, file contents, remote URLs or untrusted
instructions to private evidence.

Tests cover crash after POST before any session ID, restart from a pinned
session, double-submit protection, stale compare-and-set, session ID
replacement rejection, invalid model/session identifiers, overlapping
attempts, and a changed reviewed output scope.

This is C2.9a groundwork for the native end-to-end plan in
https://github.com/infimount/infimount/issues/234. Stable v0.8.1 remains
unchanged; do not relabel these offline tests as real API qualification.
