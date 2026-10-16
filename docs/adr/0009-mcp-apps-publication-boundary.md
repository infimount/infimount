# 0009 — MCP Apps is a review surface, not publication authority

Date: 2026-10-16

## Status

Accepted for v0.8.2 implementation.

## Context

MCP Apps can render rich interactive output reviews inside an agent host. Tool visibility controls can hide app-specific actions from a model's ordinary tool list. But a visibility annotation is not an authorization credential and cannot, by itself, prove a human interaction took place. A malicious/nonconforming host or a model with independent tool access could attempt direct publication calls.

## Decision

1. The model-visible capability may request a review UI and limited safe summary data. It must never grant write-back authority.
2. An app-only publication method is **necessary for UX and least-exposed discovery, but insufficient for security**. A host-side button, app-only metadata, or a preview token alone is not proof of human approval.
3. Final apply must be authorized by the trusted Infimount control service per ADR 0008. The service verifies a single-use, time-bounded grant bound to the exact task, reviewed output hashes, destination, conflict mode, and preview fingerprint, then revalidates under existing locks.
4. Require explicit supported MCP Apps capability before serving an interactive resource. If unavailable, degrade to review-only or open the trusted desktop flow. Never enable a less safe generic publication tool.
5. UI resources receive only bounded review summaries, hashed output descriptors, and opaque IDs. No credentials, full filesystem paths, unredacted storage configuration, secrets, or unchecked HTML from agent outputs.
6. Protect against cross-origin calls, forged UI events, replay, browser-host injection, stale views, race conditions, and capability-downgrade. A model-generated instruction to publish is not an approval.
7. Keep all write semantics from v0.8.1: nothing selected by default, exact preview, create-only fail/rename, output rehash, destination validation, verified bytes, unique receipt, and cleanup-required partial failure.
8. Cap file reads and UI payloads. Don't assume the agent host enforces our limits.
9. The desktop review path stays available and is the default trusted approval broker until a different secure human interaction channel is demonstrated.

## Rejected alternatives

- Registering `apply_task_publication` as app-only and treating the annotation as access control.
- Trusting an arbitrary host callback as a human attestation.
- Letting an agent use the general Infimount MCP storage writer to reproduce publication under the guise of an approved action.
- Bypassing the destination fingerprint on an embedded UI path.

## Consequences and tests

B3 requires adversarial tests that call app-only endpoints directly without a trusted grant, replay consumed grants, modify output bytes after review, change destination configuration, or strip app support. All fail closed. An actual supported host must also complete a real review and single approved publication without ever exposing secrets or source host paths.
