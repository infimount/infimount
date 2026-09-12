# Infimount 0.8.1-rc.5: guided Agent Workspace activation hardening

Release: not published yet.

Infimount 0.8.1-rc.5 is the fifth release candidate for Agent Tasks. It keeps the rc.4 release pipeline, Agent Task publication safety model, and installer hardening, while fixing the product workflow defects discovered during the first real rc.4 pilot.

## Why rc.5

`v0.8.1-rc.4` was published successfully. Its canonical Release workflow passed the complete release gate, multi-platform packaging, publication, and public-asset validation chain. The automatic downstream `Post Release Validation` workflow also passed, closing the rc.3 tag-propagation defect.

The resumed real Agent Tasks pilot then exposed problems before a complete three-workload run could be recorded. The issues were in Agent Workspace onboarding and guided MCP activation, not in release plumbing:

- a legacy Local Filesystem storage configured with `~` could browse normally but still fail when first bound to an Agent Workspace namespace;
- onboarding required a normal user to understand the full Advanced MCP administration surface before a workspace became usable;
- closing storage, workspace, or MCP dialogs could unexpectedly relaunch onboarding because modal state was coupled to broad application state changes;
- the client-adapter step could exceed the usable desktop viewport without a reliable internal scroll region;
- workspace readiness needed to remain specific to the selected workspace rather than being inferred from shared storage exposure;
- guided activation needed to fail closed when an already-running MCP configuration carried extra global tools or a non-loopback HTTP listener.

PR #102 addresses those pilot-discovered product blockers. Because these are post-rc.4 product behavior changes, rc.5 is required before stable promotion.

## Agent Workspace storage binding

Local Filesystem workspace creation now aligns with the Local Filesystem operator's existing home-alias behavior:

- legacy `~`, `~/...`, and `~\\...` roots are accepted for workspace preparation;
- before the first workspace binds that storage namespace, Infimount resolves the current user's home directory, verifies the resolved directory exists, canonicalizes it, and persists the canonical root;
- normalization is refused if a workspace is already bound to that storage, preventing namespace identity from being rewritten underneath an existing workspace;
- shell-variable roots such as `$HOME/...` remain invalid and must be corrected explicitly;
- ordinary relative roots remain invalid;
- root alias precedence matches the backend operator so UI eligibility and namespace binding cannot disagree about which configured root is authoritative.

The normal workspace UI remains backend-generic. Agent Workspace creation is not restricted to Local Filesystem merely because Agent Tasks v1 currently are.

## Guided Agent Access

Onboarding now contains an explicit **Agent Access** step between Workspace and Client setup.

The simple path is deliberately narrow:

- it first performs a read-only backend preflight of the selected workspace schema, namespace binding, workspace-managed path rule, storage state, and validated access profile;
- first-time setup enables local stdio rather than activating a previously drafted HTTP bind;
- read-only workspaces enable only the read tools;
- read-write workspaces add only `mkdir` and `write_file` to those read tools;
- the access profile used for runtime tool selection comes from the validated backend workspace record, not a caller-supplied UI value;
- after runtime settings are updated, the backend repeats the policy and namespace checks under the configuration lock before exposing the storage;
- one prepared workspace never marks a different workspace ready, even when both share one backing storage.

The guided path fails closed instead of silently inheriting advanced capability:

- broad default storage MCP access is rejected;
- additional manual storage grants are rejected;
- an already-active MCP server with tools outside the exact guided profile is rejected;
- an already-active HTTP server bound beyond loopback is rejected.

Those configurations remain supported through **Advanced MCP settings**, where the user can review the broader exposure explicitly.

## Onboarding and dialog behavior

The activation flow is now:

**Storage → Workspace → Agent Access → Client → Verify**

Dialog lifecycle is explicit:

- Add Storage, Agent Workspaces, and Advanced MCP return to onboarding only when onboarding opened them;
- sidebar-initiated dialogs no longer redirect into onboarding when they close;
- storage-list changes no longer automatically reopen onboarding;
- onboarding auto-open is handled once from persisted onboarding state rather than being retriggered by unrelated storage mutations.

## Desktop viewport behavior

Shared dialogs are constrained to the viewport and scroll internally. The activation wizard and MCP client-adapter step also have bounded internal scrolling so installation controls remain reachable on the target desktop viewport.

## Agent Workspace and Agent Task capability boundary

Agent Workspace creation remains a storage-scoped MCP abstraction over supported writable OpenDAL backends.

Agent Tasks v1 remain narrower:

- the Agent Task workspace must be read-write;
- Agent Tasks currently require a Local Filesystem workspace because the task pipeline relies on stronger local filesystem and atomic-rename assumptions;
- task source storage remains independent and is never newly exposed by preparation;
- remote/object-backed Agent Workspaces may still be useful as scoped MCP workspaces without implying Agent Task support on those backends.

## Agent Tasks safety model retained

rc.5 does not broaden publication semantics:

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

Once published, `v0.8.1-rc.5` supersedes rc.4 as the real-pilot candidate.

The resumed pilot starts from an installed v0.8.0 environment and must cover:

- upgrade retention for application settings, storage registrations, and Agent Workspaces before any intentional post-upgrade mutation;
- the rc.5 legacy `~` root normalization path;
- the guided Storage → Workspace → Agent Access → Client → Verify flow;
- least-privilege tool selection and per-workspace readiness;
- fail-closed behavior for extra active tools and non-loopback HTTP configuration;
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

The rc.5 candidate must pass the complete automated release chain before its tag is created:

- frontend lint, typecheck, unit tests, integration tests, coverage, and Playwright UI tests;
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

- v0.8.0 remains the current stable public release while rc.5 is evaluated.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Workspace storage must be MCP-exposed before agent handoff; workspace creation alone does not expose it.
- Advanced MCP configurations with broader tools or non-loopback HTTP remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit Advanced MCP review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
