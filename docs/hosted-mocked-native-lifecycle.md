# Infimount v0.8.2 C2.9d: mocked native hosted lifecycle

Status: **private Linux implementation with a mocked provider**. This joins
the actual prepared-input reader, private execution record store, provider
response validators and real create-only task-output importer. It does not
add an HTTP client, obtain a provider key, enable a desktop Execute action,
upload user data or grant original-storage publication.

## One connected lifecycle

The new private `hosted_execution_coordinator` runs the following sequence:

1. Pin the native workspace, task and existing output directory with the
   shared no-follow directory helpers. Read the actual manifest and inputs
   through the same pinned task descriptor and build the bounded request.
   Require the pinned outputs directory to be empty, including no existing
   links or subdirectories. Reject impossible reviewed output scopes before
   starting a paid session. Successful or partially imported tasks cannot
   create another session merely by presenting another consent capability.
2. Consume a non-cloneable, non-serializable consent capability bound to the
   exact request hash, the complete reviewed brief and input/output bundle,
   workspace/task IDs, storage namespace/revision, pinned directory identity,
   and fixed provider, U.S.-only/non-ZDR, network-disabled and $1 terms.
3. Require unexpired consent after byte verification. Persist Prepared then
   Running using the existing cross-process compare-and-set store. Recheck
   expiry immediately before submission, including time spent waiting for
   evidence-store locks. A locally expired attempt makes no provider call.
4. Make exactly one create attempt. On the first session-created event,
   retain the observed ID and persist it before accepting subsequent events.
   If persistence fails, still attempt cleanup using that observed ID.
5. Require completed root-turn evidence and inspect the pinned session.
   Audit saved items and successful command exit codes, exhaust bounded
   cursor chains and bind each artifact to that session, environment, root
   turn and exact reviewed output path. Download and hash the actual bytes.
6. Confirm deletion of the same provider session before importing outputs.
   Any known session receives a cleanup attempt even after stream, evidence,
   parsing, identity, tool, pagination or download failure.
7. Check that the originally pinned task and output directories remain
   attached at the trusted paths. Import only through the original output
   descriptor, with full reviewed-scope validation, no overwrite and a fresh
   hash of each created file. Sync created files and their containing
   directories, including new nested directory entries. A path replacement
   cannot redirect the write. Recheck attachment before reporting success.
8. Persist Succeeded, measured output counts and the canonical digest over
   paths, sizes and measured hashes only after cleanup and verified import.
   The outputs still need their ordinary separate human review.

All provider operations use a private typed transport trait. There is no URL
parameter or production implementation. JSON responses and artifact bodies
arrive through bounded chunk callbacks. Events have both a count budget and
an aggregate byte budget. Ignoring a callback error cannot restore validity.
The initial coordinator caps downloaded outputs at 32 files and 8 MiB total.
The transport contract requires a 120-second run deadline and a separate
15-second cleanup deadline. A future live adapter must enforce deadlines
during I/O and authenticate the response-to-request binding. Mocked tests do
not establish live timeout, TLS, redirect or provider interoperability.

## Consent and authority limitations

There is deliberately **no production constructor** for the export consent
type. Its fixture constructor is compiled only for unit tests. Those tests
demonstrate exact-snapshot binding, expiry and consumption by Rust ownership,
not native human presence, a production consent ledger or permission to spend.
The capability is separate from all publication types and cannot authorize
writing to source/original storage.

A future desktop caller must obtain the native user action, present the exact
scope and provider terms, use native secret storage, and hold lifecycle,
configuration and workspace mutation locks across this operation. Neither a
webview boolean nor the read-only byte-verification result is that authority.
There is no headless, MCP or Tauri entry point for this coordinator.

The request wire buffer and downloaded artifact buffers are zeroized when
dropped. Existing request-model allocations and JSON parser values are not
claimed to provide complete process-memory erasure. No request, provider
error body or prompt is logged or returned as execution evidence.

## Failure and recovery

| Observed state | Durable result and next action |
| --- | --- |
| Missing, changed or expired consent before preparation | No provider call and no output write |
| Existing output, link, subdirectory or incompatible output scope | No provider call and no new execution record |
| Consent expires while writing startup evidence | Failed local attempt, no provider call |
| Create fails before an ID is observed | Running without ID, manual reconciliation, no automatic second create |
| ID observed but first durable bind fails | Cleanup is still attempted, no further provider verification or import |
| Known session cannot be deleted or deletion is unconfirmed | Running with pinned ID, recovery required, no import |
| Provider verification fails and deletion succeeds | Failed with a sanitized error code, no import |
| Task or outputs directory replacement is observed | Failed after cleanup. No writes enter the replacement. Any bytes already committed to the original tree remain for reconciliation |
| Output appears after startup or traverses a link | Failed after cleanup, existing bytes unchanged |
| Import partially commits | Explicit import-cleanup failure, uncertain bytes are never removed automatically |
| Final evidence update fails after import | Failure returned, imported bytes retained, no successful result claimed |

The existing v1 execution record has no provider-deletion receipt field.
Therefore a terminal record alone remains **insufficient proof of cleanup**.
The existing recovery UI keeps its conservative wording. Durable deletion
receipts and resumable recovery remain later work.

The shared importer now treats failure to sync created directory entries as
an import-cleanup failure. The pre-existing private record store still uses
best-effort directory synchronization. This implementation therefore does
not claim atomic or absolute power-loss durability across outputs and their
execution record.

Pinned descriptors prevent path substitution from redirecting effects.
Attachment checks detect a replacement observed before import. They do not
prevent the account owner from renaming directories after the check, and this
module does not claim an OS security boundary against that same account.

## Qualification

The unit tests use real temporary Linux files and a deterministic in-process
provider. They cover the full successful sequence, complete brief/input scope
drift, expiry at submit, ambiguous creates, immediate ID persistence, failed
ID persistence with cleanup, failed tools and cursor loops, oversized bytes,
late stream failure, deletion failure, existing and linked output sentinels,
task/output path replacement and failure of the final evidence update. They
also verify empty-output startup, repeat-attempt denial and a retained partial
import caused by a permission failure. The permission fixture requires an
unprivileged process and is skipped when tests run as root.

The existing no-follow import/export tests remain applicable. Platform
qualification, a native consent issuer, real authenticated transport and an
explicitly approved synthetic-only live pilot are still needed before live
execution can be enabled. Stable v0.8.1 and original-storage publication stay
unchanged. The feature train remains v0.8.2, tracked by
<https://github.com/infimount/infimount/issues/234>.
