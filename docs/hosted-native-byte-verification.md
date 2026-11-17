# Infimount v0.8.2 C2.9c: fresh native byte verification before hosted export

Status: **read-only, offline native export preflight**. No HTTP call,
provider login attempt, spend, uploaded input, durable export authorization,
or publication. The first live execution remains gated by C2.9 #234.

## Native preflight flow (Unix-only)

The existing desktop "Review remote sandbox export" panel displayed only
declared input SHA-256 hashes from the saved task manifest. It now offers a
second explicit action, **Verify actual prepared bytes (no upload)**, which:

1. Reuses the selected task's validated local workspace/task identity
   and separately reviewed and redacted `AgentTaskBrief`.
2. Holds Infimount's lifecycle mutation lock, registry configuration
   transaction, and workspace mutation lock throughout validation and the
   byte-read. No other UI path may provide a native path.
3. Calls the existing namespace resolver to derive the **absolute native
   workspace root** from the validated Local Filesystem storage configuration
   and the workspace's *storage-relative* path. Validates local MCP path
   confinement and the current workspace storage namespace fingerprint.
4. Executes bounded file access on a blocking worker. The shared
   `infimount_control::sandbox_local_export` verifier opens this root and
   every task/input/manifest component via pinned Unix descriptors with
   `O_NOFOLLOW`. The first manifest read is capped at 16 MiB and uses
   `O_NONBLOCK`, so no preliminary path-based read can follow a substituted
   link, block on a FIFO, or allocate an unbounded growing file. File leaves
   must be single-linked regular files.
5. Validates the actual `task-manifest.json` against the expected workspace
   and task IDs, then reads each declared input under 32-file, 8 MiB aggregate
   and 5 MiB/file limits. Rehashes **actual bytes** to match the declared
   SHA-256 values. Rejects symlinks, hardlinks, FIFOs, mutated bytes and
   changed instructions/output scope. Manifest and inputs use the same pinned
   task directory beneath the trusted workspace. The file buffers use `Zeroizing`.
6. Returns only `inputFileCount`, `inputTotalBytes`,
   `bundleMetadataSha256`, `verifiedBytes:true` and
   `exportAuthorized:false`. **No bytes, provider credentials, local paths,
   API request JSON or approval token cross to the webview.**
7. UI checks the fresh verified result against the current metadata preview
   and invalidates both results on any reviewed-brief change, including
   A → B → A edits while an operation is in flight.

On Windows and other non-Unix platforms the command explicitly denies the
operation. Windows reparse-safe and macOS platform-specific qualification
must occur before enabling this capability there.

## Critical boundary still outstanding

This is a time-of-check observation only. Same-user file changes remain
possible after the locks are released. A future native execution must
**rehash the same input bytes again immediately before a provider request**,
and bind a one-use consent to that exact request snapshot, its output scope,
data residency/retention disclosure and bounded spending. Credentials
remain in native OS keyring, and C2.9a's private session-ID evidence must
record Running before any potentially charged create POST.

Do not interpret `verifiedBytes:true` as human authorization, proof of
provider compatibility or provider-side zero data retention. The Agents API
currently has U.S.-only residency and no ZDR, irrespective of hosted sandbox
outbound network controls.

Stable v0.8.1 and original-storage publication behavior remain unchanged.

Reference: https://github.com/infimount/infimount/issues/234
