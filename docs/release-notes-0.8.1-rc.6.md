# Infimount 0.8.1-rc.6: file-browser continuation hardening

Release: not published yet.

Infimount 0.8.1-rc.6 is the sixth release candidate for Agent Tasks. It keeps the rc.5 Agent Workspace onboarding, guided Agent Access, release pipeline, and publication safety model, while fixing the file-browser pagination defect found during the resumed real rc.5 pilot.

## Why rc.6

`v0.8.1-rc.5` was published successfully. Its canonical Release workflow passed the complete release gate, multi-platform packaging, publication, public-asset re-download validation, and automatic downstream `Post Release Validation`.

The resumed real pilot then confirmed the main rc.5 Local Filesystem correction: an Agent Workspace could be created on an existing legacy `~` storage without manually rewriting that storage first.

While continuing the same real run, the file browser exposed a separate workflow defect:

- bounded directory listing correctly produced a continuation cursor;
- the desktop exposed that backend mechanism as a visible **Load more** button;
- the one-time legacy-root normalization legitimately changed the storage registry revision;
- list cursors are intentionally signed and revision-bound, so a cursor issued before that change became stale;
- clicking **Load more** surfaced the stale-cursor failure instead of refreshing transparently.

PR #105 fixes that pilot-discovered defect. Because it changes user-visible product behavior after rc.5, a new rc.6 candidate is required before stable promotion.

## Automatic directory continuation

The normal file-browser flow no longer asks users to manage backend pagination manually.

- directory listing remains bounded to the existing first-page limit;
- when the grid or table scrolls toward the end of the currently loaded entries, Infimount requests the next page automatically;
- returned entries are appended without duplicating paths already present;
- the normal desktop UI no longer displays a persistent **Load more** control;
- an `sr-only` continuation action remains as an accessibility fallback;
- grid and table scrolling both route through the same paging state and stale-response protection.

This is a presentation/workflow change only. The backend pagination limits and signed cursor model remain intact.

## Revision-stale cursor recovery

Pagination cursors remain bound to:

- the requested path;
- recursive/non-recursive mode;
- the storage registry revision;
- bounded scan progress and continuation position.

That binding is intentional: a cursor should not silently continue against a different storage namespace or query.

The desktop now treats the expected revision-change case as recoverable. If a continuation request reports that its list cursor no longer matches the current query or storage revision, Infimount:

1. discards the stale continuation attempt;
2. reloads the current directory from page one;
3. preserves the normal stale-request guards for navigation/storage changes;
4. allows scrolling to continue from the refreshed listing.

The stale cursor is never weakened or accepted after its revision changes.

## Genuine paging failures

A real backend/network paging failure is handled differently from a revision-stale cursor:

- automatic paging stops instead of retrying continuously;
- the file browser shows a compact paging failure state with an explicit **Retry** action;
- automatic loading resumes only after a user retry succeeds;
- the backend safety-limit message is shown only when traversal is actually capped and no continuation cursor remains.

This prevents infinite retry loops while keeping transient failures recoverable.

## rc.5 corrections retained

rc.6 retains the rc.5 Agent Workspace and onboarding behavior:

- legacy `~`, `~/...`, and `~\\...` Local Filesystem roots are canonically normalized once before their first workspace namespace binding;
- `$HOME/...` and ordinary relative Local Filesystem roots remain invalid;
- workspace creation remains backend-generic and capability-gated;
- onboarding follows **Storage → Workspace → Agent Access → Client → Verify**;
- dialogs return to onboarding only when onboarding explicitly opened them;
- wizard/client content remains viewport-bounded and internally scrollable;
- guided read-only Agent Access enables only read tools;
- guided read-write Agent Access adds only `mkdir` and `write_file`;
- readiness remains specific to the explicitly prepared workspace;
- broader/manual storage grants, additional active global tools, and non-loopback HTTP configurations fail closed into Advanced MCP review.

## Agent Task capability boundary

Agent Workspace creation remains a storage-scoped MCP abstraction over supported writable OpenDAL backends.

Agent Tasks v1 remain narrower:

- the Agent Task workspace must be read-write;
- Agent Tasks currently require a Local Filesystem workspace because the task pipeline relies on stronger local filesystem and atomic-rename assumptions;
- task source storage remains independent and is never newly exposed by preparation;
- remote/object-backed Agent Workspaces do not imply Agent Task support on those backends.

rc.6 does not expand that capability boundary.

## Agent Tasks publication safety model retained

rc.6 does not broaden publication semantics:

- task preparation copies only explicitly selected source files and does not mutate the source;
- source MCP exposure is not expanded by task preparation;
- output review records current size and SHA-256 fingerprints;
- publication begins with nothing selected;
- publication supports only `fail` and `rename` conflict handling;
- overwrite is not exposed;
- publication uses create-only writes;
- stale approved previews are rejected;
- committed destination bytes are verified;
- successful publication creates a unique `publish-receipt-<publication-id>.json` receipt;
- partial multi-file failure surfaces cleanup-required state instead of claiming unsafe rollback.

## Product validation

Once published, `v0.8.1-rc.6` supersedes rc.5 as the real-pilot candidate.

The resumed pilot must cover:

- a representative v0.8.0 → rc.6 installer-over-install compatibility exercise with pre-existing configuration, storage registrations, and at least one Agent Workspace;
- legacy `~` root normalization without manual storage editing;
- the guided Storage → Workspace → Agent Access → Client → Verify flow;
- least-privilege tool selection and per-workspace readiness;
- a >200-entry disposable directory proving scroll-driven continuation and no visible manual **Load more** control;
- recovery when a legitimate storage-revision change invalidates a previously issued continuation cursor;
- one real coding Agent Task;
- one real document Agent Task;
- one real data-analysis Agent Task;
- Codex handoff through Infimount MCP;
- unchanged source bytes and unchanged source MCP exposure;
- fail-on-conflict rejection;
- stale-preview rejection;
- absence of overwrite publication;
- explicit review, approval, destination verification, and publication receipt evidence.

The previously observed rc.4 → rc.5 state retention is useful compatibility evidence but does not replace the required v0.8.0 → final-candidate exercise.

See `docs/agent-tasks-pilot.md` for the complete protocol. No completed real pilot is claimed by this release preparation.

## Release validation

The rc.6 candidate must pass the complete automated release chain before its tag is created:

- frontend lint, typecheck, unit tests, integration tests, coverage, and Playwright UI tests;
- file-browser automatic-pagination and stale-cursor recovery regressions;
- Rust formatting, clippy, tests, exact MSRV validation, and coverage;
- desktop smoke and storage simulator gates;
- dependency audit, repository lint, release consistency, and zero-manual release policy;
- signing-policy checks;
- Linux, macOS, and Windows packaging;
- updater-signature verification;
- checksums, SBOM, provenance, and installer smoke checks;
- draft-release re-download validation;
- publication and published-release re-download validation;
- automatic downstream `Post Release Validation`.

## Known boundaries

- v0.8.0 remains the stable public release while rc.6 is evaluated.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Workspace storage must be MCP-exposed before agent handoff; workspace creation alone does not expose it.
- The built-in MCP HTTP transport is loopback-only. If a user has explicitly retained an unauthenticated loopback HTTP configuration, the runtime warns that it is insecure; guided first-time Agent Access continues to prefer local stdio.
- Advanced MCP configurations with broader tools remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit Advanced MCP review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
