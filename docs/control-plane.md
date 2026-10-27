# v0.8.2 B1: shared trusted control plane

Status: **B1 foundation — publication, review, evidence, fingerprint and
receipt wire contracts extracted**. The full headless service and CLI/API
have not shipped.

## Design rule

The desktop, future `infimountctl` and future local IPC API must use the
**same** prepare/review/preview/apply authority. A headless entry point is not
allowed to copy the desktop publication engine or bypass its locks.

The new workspace crate, `infimount_control`, is for trusted control-plane
contracts and orchestration. It is **not** an MCP server, agent adapter,
storage provider, or authorization token. Agents cannot call publication apply
through this crate.

## Implemented in this slice

- Moved `AgentTaskPublicationRequest`, the reviewed output type, and the
  fail/rename conflict-policy type from Tauri into
  `infimount_control::publication` with the same serialized JSON shape.
- Moved canonical reviewed-output selection validation: UUID request fields,
  bounded output count, path confinement, lowercase SHA-256, portable
  case-collision detection, deterministic output ordering, and destination
  directory normalization.
- Re-exported the shared types through the existing Tauri publication module,
  preserving the desktop UI command contract.
- Added tests for path escapes, duplicate portable paths, missing output
  selection, stable JSON field names and rejection of unknown authority fields.

## B1.2: shared read-only reviewed-output evidence

`infimount_control::publication_evidence::verify_reviewed_output_digest`
now performs the same existence, file type, byte-size, and exact SHA-256
comparison formerly in the Tauri publication planner. Desktop calls the shared
verifier after it constructs a task-relative source path and applies the
existing storage namespace/local confinement check. Rust memory-operator tests
cover exact bytes, missing paths, size changes, same-size changes, directories,
invalid output paths, and malformed SHA-256.

This helper **does not** enumerate arbitrary storage, authorize publication,
lock remote writers, or apply a write. A caller must bind the workspace, task,
storage identity and source path before using it. The authoritative publish
path still re-verifies bytes under the original mutation lock sequence.

## B1.3: shared publication plan, fingerprint and receipt contracts

`infimount_control::publication_plan` now owns plan-file/action wire types,
the staleness fingerprint shape and its exact JSON-byte SHA-256 calculation,
and the publication receipt payload shape. The desktop still owns actual
destination discovery, collision handling, locking, create-only writes,
destination-byte verification and receipt persistence.

A pinned test retains the existing preview-token digest for a known fixture,
and receipt JSON tests preserve field names while excluding source storage
identity and host paths. This is **wire compatibility, not authorization**.
A valid preview token on its own never approves publication.

## B1.4: shared read-only review wire contract

The `infimount_control::review` module contains the unchanged desktop
review request, review file and review response schemas. Tauri re-exports the
types so the command and frontend JSON remain compatible. Tests pin the legacy
camelCase shape, explicit preview-unavailable reasons and the absence of any
publication authority fields. Unknown review request fields are rejected.

The desktop still owns workspace binding, manifest checks, local path
confinement, bounded recursive output enumeration, exact SHA-256 hashing
and preview re-verification. This slice introduces no new review endpoint,
agent tool, approval or publication behavior.

## B1.5: shared fail-closed output enumeration and byte caps

`infimount_control::review_evidence` lists task output files using the
existing OpenDAL recursive traversal. It rejects potentially truncated
10,000-entry listings, limits review to 1,000 files and 2 GiB of listed
bytes, sorts paths deterministically, and rechecks the 2 GiB cap against
actual hashed bytes. Tests cover nested listings, directory filtering,
truncation, file-count ceilings, size ceilings and integer overflow.

The desktop still validates its bound task manifest, storage namespace,
output directory, and every returned local path before hashing or previewing
each file. The same 20-preview and 64 KiB per-preview limits remain in the
desktop implementation. The helper is read-only evidence collection, not
an approval, a publication token, or an agent-facing capability.

## B1.6: shared read-only destination choice and collision planning

`infimount_control::publication_destination` now owns the same OpenDAL
existence checks, create/conflict/rename choice, bounded legacy " copy" suffix
search and case-insensitive collision reservations used by the desktop planner.
Memory-operator tests cover existing and missing objects, fail versus rename
policies, exact suffixes, dotfiles, path-length failures and non-mutating
read-only planning.

**Not publication authority:** the desktop still resolves task/workspace
identity, checks backend capabilities, validates storage namespaces and
destination confinement, rejects reserved task paths, rehashes reviewed bytes,
and replans under lifecycle/configuration/workspace locks immediately before
create-only writes and receipt persistence. This does not create an MCP tool,
headless apply path or new UI.

## B1.7: common publication context and trust binding

`infimount_control::publication_context::resolve_publication_context` now
performs the same sequential workspace schema/access checks, storage enabled/
writable checks, Local Filesystem task-workspace restriction, canonical
namespace fingerprint matching, destination identity checks, create-only
operator capability guards and task/destination path confinement previously
implemented in the desktop publication command. Trusted lookups are injected
by the desktop's local registry and operator cache; the future local CLI/IPC
must provide the same trusted bindings rather than reimplement these checks.
A new shared `PublicationContext` carries the fully resolved identities
and operator handles. Regression tests verify rejection of stale identities,
unsupported workspace schemas, disabled destinations, malformed destination
paths and valid local binding.

**Locks and authority remain unchanged.** The desktop still takes lifecycle,
configuration and workspace mutation locks in the same order and holds them
through re-resolution, replan, exact byte verification, create-only writes and
receipt persistence. This resolver is not exposed as an agent-visible tool or
an authorization bypass.

## B1.8: shared trusted read-only publication planner

`infimount_control::publication_engine` now owns the end-to-end read-only
publication preview plan: validation of the selected reviewed outputs, trusted
context resolution, bounded manifest reads, source hash re-verification,
reserved workspace and canonical namespace overlap rejection, create/conflict/
rename decisions, path collision prevention, exact size limits, sorting and
the legacy JSON-byte preview fingerprint. The desktop only injects trusted
registry/operator lookups and retains the existing Tauri response contract.

Filesystem-backed tests verify that planning never creates a destination, a
same-size source modification invalidates review evidence, conflict policies
and suffixes remain stable, and malformed manifests/reserved control paths
fail closed. Existing alias-containment tests remain available to the desktop.

**No authority moved:** the desktop alone retains lifecycle/configuration/
workspace mutation locks, the re-plan comparison during apply, create-only
destination writes and integrity-checked publication receipts. The shared
planner does not approve or apply a publication, expose new MCP tools or
require agent setup for ordinary file browsing.

## B1.9: shared locked-plan checks and create-only receipt persistence

`infimount_control::publication_apply` now owns the exact stale-preview
and unresolved-conflict checks that run after the desktop rebuilds a publication
plan under lifecycle, configuration and workspace mutation locks. It also
owns capability-gated parent directory creation and integrity-checked,
create-only receipt persistence (including abort and ambiguity handling).
Tests verify that stale review or unresolved conflicts fail closed, receipts
are not overwritten on replay, and invalid parent paths are rejected.

These helpers do **not** constitute authorization. The desktop remains the
only trusted caller of the publication apply transaction and retains its
lock order, source rehashing, destination confinement, create-only file
publication, cleanup policy, receipt path binding and final result contract.
No CLI, IPC or agent-visible publication endpoint is exposed.

## B1.10 contract only: protected, single-use publication approval binding

`infimount_control::publication_approval` defines the future protected-store
approval record and pure binding checks. The record binds a trusted actor,
workspace/task identity, exact reviewed file hashes and destination paths,
destination storage ID, namespace fingerprint and revision, conflict mode,
preview fingerprint, creation/expiry and a one-use nonce. Records are capped at
15 minutes, fail on revocation or prior consumption, reject unknown fields,
and must match freshly replanned source/destination state. Tests cover expiry,
replay, actor substitution, namespace/revision drift, conflicts, changed bytes
and altered output paths.

**Not yet enabled:** This is a validation contract, not a grant issuer or an
authority to publish. It does **not** load, authenticate, store or atomically
consume approvals. A future server-owned persistent grant store outside
agent-visible paths must provide those guarantees under the existing locks
before headless publication is implemented. Neither a preview token nor a
serialized approval record is sufficient permission. Desktop publication is
unchanged and remains the only existing apply entry point.

## B1.11: shared reviewed-file commit loop under desktop locks

`infimount_control::publication_apply::publish_locked_reviewed_files` now
co-locates revalidation of the reviewed preview fingerprint/conflict count
with the existing create-only reviewed-file copy loop. It verifies source
paths and hashes, destination path confinement and committed destination
hashes, and retains partial-commit `TransferCleanupRequired` behavior
without unsafe rollback by deletion.

Filesystem-backed tests prove exact bytes, source drift rejection, stale or
conflicting plan rejection before any write, and late second-destination
collision with the earlier published file and occupied sentinel preserved.

Failures on a later file, including destination validation or parent creation
before opening its writer, report `TransferCleanupRequired` if an earlier file
has already committed. Existing committed output and unrelated sentinels remain
untouched.

This remains a privileged Rust helper, **not publication approval**. Only
the existing Tauri command invokes it after re-planning under lifecycle,
configuration and workspace locks. B1.12 consolidates receipt persistence
into the same helper. No CLI or MCP publication endpoint is enabled.

## B1.12: one shared, verified output-and-receipt commit transaction

The existing desktop command retains operational-state checks, preview token
shape validation, lifecycle/configuration/workspace locks and authoritative
plan rebuilding. It now calls a single trusted control helper that verifies
the request and conflict policy match the rebuilt plan fingerprint, commits each reviewed file create-only,
then creates and integrity-verifies the unique publication receipt.

Any failure after committed destination bytes, including receipt validation,
serialization, collision or write verification, returns
`TransferCleanupRequired` without deleting potentially replaced paths.
A deterministic receipt collision test and a full exact-byte receipt/response
test protect this boundary. The desktop still issues receipt IDs/timestamps
only after source publication succeeds, preserving the legacy API and timing.

The underlying per-file commit loop is crate-private, so external trusted
callers must use the shared output-and-receipt orchestration rather than
silently omitting the receipt.

**No new authorization:** This remains an internal privileged Rust service.
It accepts no model-created approval as authority and is invoked only through
the existing desktop-human-mediated command under its original lock order.
A protected, single-use approval store remains mandatory before any B2
headless publication entry point.

## B1.13b: live destination revision approval snapshot (read-only)

`snapshot_from_rebuilt_publication_plan` validates the exact reviewed request
against the rebuilt plan and derives a future approval-validation snapshot from
trusted workspace/task, conflict policy, checked file list, preview fingerprint
and the **live destination StorageRecord.revision**. The caller must hold the
existing lifecycle/configuration/workspace mutation locks and resolve the
authoritative registry state. Conflicted plans, uninitialized revision zero and
the saturating `u64::MAX` revision are rejected before a grant could be accepted.

A simulated change-away/change-back test shows that a revision increase rejects
an earlier grant even when the destination namespace and preview fingerprint
are unchanged. This assumes the storage revision remains authoritative and
monotonic. Configuration imports/restores and out-of-band registry edits still
require explicit audit and fail-closed handling before actual headless apply.
No approval issuer, persistence, consumption or new publishing endpoint ships.

## B1.13c1: fail-closed revision exhaustion

Storage updates and legacy secret migrations now use
`next_storage_revision` with a checked increment instead of
`saturating_add(1)`. A revision at `u64::MAX` is rejected before
writing a replacement record or allocating a new credential reference.
Regression tests verify no wraparound, no saturation replay and no partial
desktop update for an exhausted revision.

**Not yet ABA-safe across restore/import:** a recovery restore can still
replace the registry with saved revision values, and raw rollback can restore
older bytes. Issue #185 tracks the independent trusted approval epoch and
restore/import/recovery integration. No human grants or headless publication
are enabled by this patch.

## B1.13c2: rebase successful recovery restore revisions

Recovery restore now uses an atomic registry-locked read/rebase/write operation
for storage records. For a storage ID that exists in the live registry, the
restored revision becomes one greater than the larger of the existing and
backup revisions. Repeated restoration of historical data cannot decrement or
reuse that particular revision, and duplicate IDs and overflow are rejected.
Synthetic and filesystem-backed regression tests cover restore ABA behavior.

**Not a complete approval epoch:** import, deletion/recreation of a previously
used ID and raw-byte journal rollback remain distinct cases requiring proof.
Recovery from already-malformed registry JSON stays available, but has no
trustworthy previous revision to rebase against. A future approval issuer must
treat that state as untrusted until a separate service-owned epoch is created.
This slice retains the existing backup JSON schema and never permits headless
publishing. Issue #185 stays open.

## B1.13c3: checked imported-storage revision transitions

Both legacy and preview/apply configuration imports now reject exhausted
storage revisions instead of silently saturating a changed record at the same
numeric revision. Existing unchanged records retain their revision, while
normal changes still advance by exactly one. Regression tests cover
saturation and unchanged vs changed imports.

This only closes the numeric overflow case. Import transaction rollback can
restore previous registry bytes. Pending headless grants must be invalidated
by a separate process-owned approval ledger under the same authoritative
locks. No production grant issuer or headless publication exists.

## B1.13c3: checked imported-storage revision transitions

Both legacy and preview/apply configuration imports now reject exhausted
storage revisions instead of silently saturating a changed record at the same
numeric revision. Existing unchanged records retain their revision, while
normal changes still advance by exactly one. Regression tests cover
saturation and unchanged vs changed imports.

This only closes the numeric overflow case. Import transaction rollback can
restore previous registry bytes. Pending headless grants must be invalidated
by a separate process-owned approval ledger under the same authoritative
locks. No production grant issuer or headless publication exists.

## B1.13c4: checked workspace-policy and agent-access revisions

Workspace-managed MCP policy mutations (grant, removal and targeted rollback)
and the desktop Agent Access binding/exposure paths now use checked storage
revision increments. This rejects exhausted counters before the mutation is
saved and prevents a saturating ABA replay. A filesystem-backed regression
proves a failed workspace policy update leaves existing policy and revision
unchanged.

A persisted numeric revision still is **not** an approval authority across
raw journal rollback, delete/recreate or unrestricted same-user file access.
No new grant issuer, CLI or agent-visible publication endpoint is created.

## B1.13d prototype: process-private one-use approval ledger

`EphemeralPublicationApprovalLedger` is a memory-only trust-state prototype
with **no production grant issuer**. The only fixture insertion API is compiled
under `cfg(test)`. Under its mutex, a matching approval can be consumed at most
once before output I/O, and a mismatch permanently revokes the pending grant.
Tests cover concurrent attempts, replay after an output failure, expiry, actor
and revision drift, configuration invalidation, duplicate nonce and restart.

A new process begins with an empty ledger. No grant record or nonce is
serialized into the user-accessible task workspace or backup. This avoids
promising that persisted `StorageRecord.revision` is a durable authorization
epoch across backup/raw rollback.

**Not connected:** The existing desktop apply command is unchanged. There
is no user-presence proof, grant issuance service, config-mutation hook wiring,
cross-process authority owner, IPC or CLI publish entry point. Issue #181 must
prove these before any headless apply can ship.

## Trust and behavioral compatibility

The **same** Tauri handlers still own `preview_agent_task_publication` and
`publish_agent_task_outputs`. They still acquire the lifecycle/configuration/
workspace locks in their existing order, rebuild publication plans, compare
preview fingerprints, verify exact source/output hashes, enforce create-only
fail/rename semantics, and write verified unique receipts.

No model-visible publication tool, new external endpoint, permission
bypass, default client adapter or release tag is introduced. The existing
v0.8.1 104-argument Codex fixture and the A4 opt-in Claude pilot are unchanged.

## Next B1 slices

1. Bind future control-service lock ownership and authenticated actor authority
   explicitly. Do not permit a new caller to invoke privileged apply mechanics
   without mandatory guards.
2. Build the server-owned, single-use human approval store with authenticated
   actors, protected persistence, expiry, revocation and atomic consumption
   under the existing locks before headless apply.
3. Only after the protected approval store and apply parity tests, build
   the B2 CLI against that same trusted service.
   Headless publication must require explicit, server-controlled human
   approval or a narrowly scoped audited delegation; a `--yes` switch is
   not sufficient.

See [ADR 0008](adr/0008-trusted-headless-control-plane.md) and the
[v0.8.2 roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).
