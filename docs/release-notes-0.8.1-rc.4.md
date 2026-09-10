# Infimount 0.8.1-rc.4: post-release validation hardening

Release: not published yet.

Infimount 0.8.1-rc.4 is the fourth release candidate for Agent Tasks. It keeps the Agent Tasks, Agent Workspace, installer, and Linux release behavior from rc.3 unchanged and adds only the post-release validation fix merged in PR #99.

## Why rc.4

`v0.8.1-rc.3` was published successfully. Its canonical Release workflow completed successfully across all release gates, Linux/macOS/Windows builds, updater-signature verification, checksums, SBOM/provenance generation, installer smoke checks, draft validation, publication, and the release job's own re-download/validation of the public assets.

The separate `Post Release Validation` workflow then failed before artifact validation. The workflow correctly resolved and checked out `v0.8.1-rc.3`, but `sync-release-version.mjs` read `GITHUB_REF_NAME=main` from the `workflow_run` event context. GitHub reserves `GITHUB_*` variables, so the workflow step's attempted `env:` override did not replace that value. The script therefore rejected `main` as an unsupported release tag.

This was classified as release-automation orchestration, not a product or published-artifact defect. rc.3 remains immutable and published. rc.4 carries the fix so the complete automatic `Release -> Post Release Validation` handoff can be exercised by a fresh candidate.

## Post-release validation fix

PR #99 hardens release-tag propagation in three layers:

- `Post Release Validation` resolves the release tag once and passes that exact tag explicitly to `sync-release-version.mjs`;
- the shell invocation also assigns `GITHUB_REF_NAME` directly for compatibility with already-published candidate code paths;
- `sync-release-version.mjs` now prefers an explicit CLI tag argument and falls back to `GITHUB_REF_NAME` for normal tag-push release jobs;
- the release-version model regression test reproduces a `workflow_run` context with `GITHUB_REF_NAME=main` and verifies that the explicit release tag wins;
- the regression check is part of `test:docs`, so this orchestration path is covered by normal repository validation.

The validator still fails closed for malformed or nonexistent release tags.

## Linux release dependency hardening

rc.4 retains the rc.3 APT-source hardening that isolated release builds from the unrelated Google Chrome repository issue observed during rc.2:

- release jobs use `scripts/ci-apt-install.sh` rather than raw repeated APT refresh/install sequences;
- `scripts/prepare-ci-apt-sources.sh` disables only the unrelated Google Chrome source on GitHub-hosted Ubuntu runners;
- Ubuntu and other configured package sources are preserved;
- source isolation is idempotent;
- package-index refresh uses bounded retries and still fails closed if required repositories remain unhealthy.

## Agent Workspace creation

rc.4 retains the boundary-first Agent Workspace model introduced after the first real pilot attempt:

- new Agent Workspaces are plain scoped folders, not coding/research/data-analysis agent templates;
- creation asks for workspace name, storage, derived storage-relative location, and read/write choice;
- the workspace folder is derived under `/agent-workspaces/<workspace-name>` rather than requiring a second host path;
- managed workspace MCP policy is part of normal creation;
- read-only remains the safe default and agent writes require explicit opt-in;
- existing templated workspaces remain compatible;
- Local Filesystem storage used for workspace namespace binding requires an explicit absolute host path; shell-style `$HOME/...`, `~/...`, missing, and relative roots are rejected before creation.

## Installer clarity

rc.4 retains the Linux installer PATH-shadow warning. If another `infimount` executable earlier in `PATH` would continue to launch after package installation, the installer reports the conflicting resolved path instead of silently claiming an unambiguous installation.

## Agent Tasks safety model

rc.4 does not broaden the Agent Tasks authorization surface:

- task preparation copies only explicitly selected source files and never mutates or newly exposes source storage;
- Codex handoff uses the existing task/workspace MCP boundary;
- output review records fresh size and SHA-256 fingerprints;
- publication begins with nothing selected and requires explicit output, destination, and conflict-policy selection;
- publication supports only `fail` or `rename`, exposes no overwrite mode, uses create-only writes, and verifies committed destination bytes;
- stale publication previews are rejected;
- successful publications write unique create-only `publish-receipt-<publication-id>.json` receipts;
- partial multi-file failures surface cleanup-required state instead of claiming atomic rollback.

See `docs/agent-tasks.md` and `docs/agent-workspaces.md` for the complete product contract.

## Product validation

Once published, `v0.8.1-rc.4` supersedes rc.3 as the candidate for the real Agent Tasks pilot. The pilot should start from an installed v0.8.0 environment and exercise:

- upgrade retention for application configuration, storage registrations, and Agent Workspaces;
- one real coding task;
- one real document task;
- one real data-analysis task;
- Codex handoff through Infimount MCP;
- source non-mutation and unchanged source MCP exposure;
- fail-on-conflict rejection;
- stale-preview rejection;
- absence of overwrite publication;
- explicit review, approval, destination verification, and publication receipt evidence.

No real pilot completion is claimed by this release preparation. Synthetic fixtures and automated release tests remain test evidence only.

## Release validation

The rc.4 candidate must pass the complete automated release chain: frontend lint/typecheck/unit/integration/coverage, Playwright UI tests, Rust format/clippy/tests/coverage, desktop smoke, storage simulator tests, dependency audit, release consistency, signing policy, multi-platform packaging, updater-signature verification, checksums, SBOM, provenance, installer smoke checks, draft-release re-download validation, publication, published-release re-download validation, and the downstream `Post Release Validation` workflow.

For rc.4 specifically, successful automatic downstream validation is part of the candidate objective because that is the only behavior changed from rc.3.

## Known boundaries

- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Workspace storage must be exposed to MCP before Codex handoff; workspace creation itself does not implicitly expose a storage.
- Local Filesystem storage used for workspace namespace binding must use an explicit absolute host path.
- Agent Task publication deliberately has no overwrite mode.
- Existing local symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- v0.8.0 remains the current stable release while rc.4 is evaluated. Stable public identity must not move to v0.8.1 until a stable release is separately prepared and published.
