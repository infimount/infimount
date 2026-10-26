# 0011 — Session-bound publication approval authority

Date: 2026-10-25

## Status

**Proposed, security gate not satisfied.** This ADR is not authorization to
enable headless apply, add an approval issuer or publish from MCP/CLI/IPC.
It refines [ADR 0008](0008-trusted-headless-control-plane.md), issue #181 and
the destination-revision audit in issue #185.

## Problem

Storage record revisions increase on ordinary edits, but a backup restore,
raw-byte recovery rollback or deletion/recreation can reintroduce a previous
numeric revision and namespace fingerprint. A preview hash can also repeat
when bytes/configuration change away and then back. Neither field can become
a globally monotonic authority without redesigning every persistence path.

An agent with unrestricted native shell access and the same OS identity as
the desktop may also write directly to destination files or user-owned
credentials. A local-only socket, file mode 0600 and a WebView button do
not establish the human's physical presence or isolate such an agent.

## Proposed v0.8.2 first cut

**No durable or unattended publication approvals.** The desktop remains
browser-first and uses the current human-mediated publication flow.

A future control service may support headless **preview/dry-run** without a
grant. To perform a headless write it would need all of the following:

1. One local trusted process owns the approval state and journal recovery.
   Every process boot creates a new private random process identity. No
   approval, nonce or key capable of authorizing apply is in a task tree,
   MCP-visible storage or a user-editable JSON grant file.
2. A native platform user-verification ceremony must be independently
   qualified on each supported OS. A same-user CLI invocation, `--yes`,
   shell possession, browser click or model-supplied message is not proof.
   Enrollment must not permit an agent to register a new human credential.
3. Display the **current authoritative** task, selected paths, source hashes,
   destination, conflict decisions and revision/policy to the human.
4. Acquire locks in the existing order:
   lifecycle → configuration transaction → workspace mutation.
   Re-resolve storage and workspace identity, rehash output bytes and rebuild
   the trusted preview while locks remain held.
5. Rebind the verified actor, native user-presence assertion, private
   process identity, one-use nonce and fresh plan to a pending grant.
   Prefer immediate consumption during this same bounded transaction.
   The consent window must be finite and must never stall ordinary browsing.
6. Atomically claim/consume the in-memory grant **before** the first
   create-only destination write. Consumption is irreversible even if a
   later output/receipt fails or the caller disconnects.
7. Mark all pending grants revoked before any storage/workspace mutation,
   import, restore, rollback or journal recovery within that process.
   If invalidation cannot be proven, reject headless apply. Restart erases
   all grants. Do not recover pending approvals from backup or disk.

The application must keep its existing verified create-only copy and
receipt transaction and partial-commit cleanup semantics.

## Threat boundary

This protects the **Infimount-mediated publish path** only. To protect the
underlying storage from an arbitrary same-UID process, agent execution must
be OS-isolated and deprived of ambient destination credentials. Authorization
cannot be obtained from the preview fingerprint or from a serializable
`PublicationApprovalRecord` alone.

If a native user-presence ceremony is unavailable or cannot bind a trusted
credential to the request, headless preview may remain supported but
headless publication must be disabled. Long-lived and automatic delegated
grants remain out of scope.

## Validation before moving to Accepted

- Prove real native user verification and secure enrollment/recovery on
  Linux, macOS and Windows, including unsupported-device fail-closed paths
- Attempt counterfeit issuer/API calls from an arbitrary same-user terminal
  agent and prove they cannot mint, inspect or replay a grant
- Prove one winner for parallel claims, no grant after restart and no replay
  after failed/partial output, receipt failure or process crash
- Prove all configuration/workspace mutation, import, restore, rollback and
  recovery paths revoke pending grants before changing state, even if the
  storage ID/namespace/revision/preview later return to prior values
- Prove the existing lock lifetime and browser-only UI are unaffected
- Require every exact-head CI workflow including desktop smoke, pilot,
  Rust 1.94 MSRV, audit, coverage and browser regressions

`EphemeralPublicationApprovalLedger` in B1.13d is **only a unit-tested
concurrency prototype**. It has no production grant issuer, native presence
proof, mutation-hook wiring or executable headless apply path.
