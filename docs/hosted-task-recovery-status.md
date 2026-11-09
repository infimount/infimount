# Infimount v0.8.2 C2.9b: read-only hosted recovery status

Status: **read-only native desktop visibility into private execution evidence**.
No provider HTTP, credential retrieval, spend, new execution, output import,
session deletion, or original-storage publication.

The shared `ExecutionRecordStore` now supports a strictly bounded
`list_for_task(workspace_id, task_id)` query. It:

- Acquires the same cross-process private evidence mutation lock used by
  creation and transitions, and requires canonical UUIDs.
- Validates each ancestor of the private task history as an actual directory,
  rejecting symlinks and reparse points, and rejects unfamiliar or damaged
  entries rather than silently dropping ambiguous active attempts.
- Caps the read at 100 records, requires every record ID and task/workspace
  identity to match its private filename and directory, and sorts the
  validated entries newest-first.

The `list_hosted_task_execution_status` native Tauri command first
validates the selected Agent Task and workspace using the existing Local
Filesystem review context and its task manifest, then reads private evidence
on a blocking worker (not a Tauri async worker) and filters only
`OpenaiSandbox` private execution records.

The response exposes only execution UUID, timestamps, status and a sanitized
recovery classification. **It never exposes an opaque provider session ID,
the account API key, original source path, model instructions, or outputs.**

The committed Agent Workspace Tasks view now includes a read-only Hosted
Execution History panel with a refresh control. It uses the saved private
record, not a browser completion flag, to distinguish:

- `requires_fresh_approval`: prepared attempt, not yet sent
- `manual_session_reconciliation`: running with unknown provider session ID;
  it may have been charged, so NEVER blindly retry create
- `inspect_pinned_session`: a provider session ID is privately recorded;
  inspect that exact session and perform authenticated cleanup
- `terminal_needs_cleanup_review`: finished attempt, but the v1 record
  does not yet prove provider-side deletion. Terminal does NOT imply cleanup.

The panel has no execute, retry, delete-provider-session or publish button.
The first real hosted execution remains unqualified and requires C2.9's
native consent, credential binding, one-shot create, bounded transport,
cleanup and verified import. See issue #234.

Stable v0.8.1 unchanged. These tests are offline; they do not constitute
paid live-provider interoperability qualification.
