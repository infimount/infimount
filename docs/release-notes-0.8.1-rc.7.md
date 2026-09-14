# Infimount 0.8.1-rc.7: dismissible activation reminder

Release: not published yet.

Infimount 0.8.1-rc.7 is the seventh release candidate for Agent Tasks. It keeps the rc.6 Agent Workspace onboarding, file-browser pagination recovery, guided Agent Access, release pipeline, and publication safety model, while correcting the skipped-activation reminder behavior discovered during resumed product validation.

## Why rc.7

`v0.8.1-rc.6` was published successfully. Its canonical Release workflow passed the release gates, multi-platform packaging, publication, public-asset re-download validation, and the automatic downstream `Post Release Validation` workflow.

The resumed validation flow then exposed a user-facing defect after **Skip for now**:

- activation correctly remained incomplete;
- the application correctly kept a reminder that Agent Access was still unverified;
- the reminder exposed **Finish setup** but no dismiss action;
- as a result, a user who had explicitly chosen to defer setup could not remove the reminder for the rest of the app lifetime.

PR #107 fixes that behavior. Because it changes user-visible product behavior after rc.6, a new rc.7 candidate is required before stable promotion.

## Session-only reminder dismissal

The incomplete-activation reminder now has two explicit actions:

- **Finish setup** resumes the activation wizard;
- the dismiss control hides the reminder for the current app lifetime.

Dismissal is intentionally not persisted as activation completion. It does not:

- mark onboarding complete;
- mark Agent Access verified;
- change MCP settings;
- change storage or workspace policy;
- suppress the reminder permanently.

If activation is still incomplete on a later app launch, the reminder can appear again.

This keeps the safety signal while respecting the user's explicit choice to defer setup.

## Automated regression coverage

The rc.7 reminder behavior has dedicated component tests proving that:

- **Finish setup** still invokes the resume path;
- dismissing hides the reminder without mutating completion state;
- remounting the reminder makes it visible again, matching the intended non-persistent dismissal model.

The reminder is wired into the existing `onboardingSkipped && !onboardingCompleted` condition, so completed activation still suppresses it entirely.

## rc.6 corrections retained

rc.7 retains the rc.6 file-browser corrections:

- normal directory continuation is scroll-driven rather than a visible manual **Load more** workflow;
- continuation entries append without duplicate paths;
- revision-stale cursors cause a current-directory refresh instead of surfacing a generic continuation error;
- genuine paging failures stop automatic retries and expose an explicit retry state;
- stale responses remain guarded after navigation or storage changes.

## Agent Workspace and guided-access corrections retained

rc.7 retains the rc.5 Agent Workspace and onboarding behavior:

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

rc.7 does not expand that capability boundary.

## Publication safety model retained

rc.7 does not broaden Agent Task publication semantics:

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

Once published, `v0.8.1-rc.7` supersedes rc.6 as the real-pilot candidate.

The resumed pilot must cover:

- a representative v0.8.0 → rc.7 installer-over-install compatibility exercise with pre-existing configuration, storage registrations, and at least one Agent Workspace;
- legacy `~` root normalization without manual storage editing;
- the guided Storage → Workspace → Agent Access → Client → Verify flow;
- least-privilege tool selection and per-workspace readiness;
- the **Skip for now** reminder showing both **Finish setup** and a dismiss control;
- session-only reminder dismissal without falsely completing activation;
- reminder return after a later app launch while activation remains incomplete;
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

See `docs/agent-tasks-pilot.md` for the complete protocol. No completed real pilot is claimed by this release preparation.

## Release validation

Before creating the `v0.8.1-rc.7` tag, the exact final release-preparation PR head must pass all six pre-tag workflows:

- CI;
- Integration Tests;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint.

Those gates cover frontend lint, typecheck, unit/integration/UI tests and coverage; activation-reminder and file-browser regressions; Rust formatting, clippy, tests, exact MSRV and coverage; desktop smoke; storage simulator checks; dependency audit; repository consistency; and release-policy rehearsal.

After the tag is created, the canonical Release workflow must then pass the tag-triggered release chain, including:

- signing-policy checks;
- Linux, macOS, and Windows packaging;
- updater-signature verification;
- checksums, SBOM, provenance, and installer smoke checks;
- draft-release re-download validation;
- publication and published-release re-download validation.

The automatic downstream `Post Release Validation` workflow must also pass before rc.7 is accepted as the candidate for the resumed real pilot.

## Known boundaries

- v0.8.0 remains the stable public release while rc.7 is evaluated.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Workspace storage must be MCP-exposed before agent handoff; workspace creation alone does not expose it.
- The built-in MCP HTTP transport is loopback-only. If a user has explicitly retained an unauthenticated loopback HTTP configuration, the runtime warns that it is insecure; guided first-time Agent Access continues to prefer local stdio.
- Advanced MCP configurations with broader tools remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit Advanced MCP review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
