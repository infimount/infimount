# Infimount 0.8.1-rc.3: release pipeline hardening

Release: not published yet.

Infimount 0.8.1-rc.3 is the third release candidate for Agent Tasks. It carries the same Agent Tasks and Agent Workspace product behavior validated for rc.2 and adds only release-infrastructure hardening required after rc.2 could not complete Linux packaging on GitHub-hosted runners.

## Why rc.3

`v0.8.1-rc.2` was tagged at the intended candidate commit and all product/release gates completed successfully, including frontend, Playwright, Rust, desktop smoke, storage simulator, release consistency, signing policy, and the macOS and Windows platform builds.

The Linux platform build failed before Infimount compilation during `apt-get update`. GitHub-hosted Ubuntu runners contained an unrelated Google Chrome APT source whose release metadata and `Packages.gz` content disagreed, producing a `Hash Sum mismatch`. A retry on a different runner failed identically. The dependent publish job therefore remained skipped and no v0.8.1-rc.2 GitHub Release was published.

This was classified as a release-environment/infrastructure failure, not an Agent Tasks product defect. rc.2 remains immutable and unpublished; rc.3 is a new candidate rather than a retag of rc.2.

## Linux release dependency hardening

- Release jobs now use `scripts/ci-apt-install.sh` instead of raw `apt-get update`/install sequences.
- Before refreshing package indexes, `scripts/prepare-ci-apt-sources.sh` disables only the unrelated Google Chrome source preinstalled on GitHub-hosted Ubuntu runners.
- Ubuntu package sources and other unrelated configured sources are left unchanged.
- Disabled Chrome source files use the `.infimount-disabled` suffix and the operation is idempotent; already-disabled files are not renamed again.
- Package-index refresh has a bounded three-attempt retry loop and clears partial APT list downloads between retries.
- The release Rust gate, desktop smoke gate, Linux packaging build, RPM extraction fallback, and publish-job `jq` installation all use the same wrapper.
- A deterministic fixture is part of `test:docs` and verifies Chrome-only isolation, preservation of Ubuntu/Microsoft sources, idempotency, bounded retries, and wrapper installation behavior.

This hardening does not allow package-index errors to be ignored. If the required Ubuntu repositories remain unhealthy after the bounded retries, the release still fails closed.

## Agent Workspace creation

rc.3 retains the boundary-first workspace model introduced for rc.2:

- New Agent Workspaces are plain scoped folders rather than coding/research/data-analysis agent templates.
- Creation asks for workspace name, storage, derived storage-relative location, and read/write choice.
- The workspace folder is derived under `/agent-workspaces/<workspace-name>` instead of requiring a second host path.
- The managed workspace MCP rule is part of normal creation.
- Read-only remains the safe default; agent writes require explicit desktop opt-in.
- Existing current-schema templated workspaces retain their existing starter-note/checkpoint compatibility behavior.
- Local Filesystem storage used for workspace namespace binding must use an explicit absolute host path; `$HOME/...`, `~/...`, missing, and relative roots are rejected before creation.

## Installer clarity

rc.3 also retains the Linux installer shadowing warning added after the first pilot setup attempt. If another `infimount` executable earlier in `PATH` would continue to launch after package installation, the installer reports both paths instead of silently claiming an unambiguous installation.

## Agent Tasks safety model

rc.3 does not broaden the Agent Tasks authorization surface:

- task preparation copies only explicitly selected source files and never mutates or newly exposes source storage;
- Codex handoff uses the existing task/workspace MCP boundary;
- output review records fresh size and SHA-256 fingerprints;
- publication begins with nothing selected and requires explicit output, destination, and conflict-policy selection;
- publication supports only `fail` or `rename`, has no overwrite mode, uses create-only writes, and verifies committed destination bytes;
- stale publication previews are rejected;
- successful publications write unique create-only `publish-receipt-<publication-id>.json` receipts;
- partial multi-file failures surface cleanup-required state instead of claiming atomic rollback.

See `docs/agent-tasks.md` and `docs/agent-workspaces.md` for the complete product contract.

## Product validation

`v0.8.1-rc.3` supersedes rc.2 as the candidate for the real Agent Tasks pilot. The pilot should start from an installed v0.8.0 environment and exercise:

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

Neither rc.1 nor rc.2 is completed real-pilot evidence. rc.1 reached workspace setup and exposed the workspace workflow defect fixed in rc.2. rc.2 never became a published installable release because its Linux release job was blocked by the external APT repository inconsistency.

No real pilot completion is claimed by this release preparation. Synthetic fixtures and automated release tests remain test evidence only.

## Release validation

The candidate remains subject to the complete zero-manual automated release chain: frontend lint/typecheck/unit/integration/coverage, Playwright UI tests, Rust format/clippy/tests/coverage, desktop smoke, OpenDAL simulator tests, dependency audit, release consistency, signing policy, multi-platform packaging, updater-signature verification, checksums, SBOM, provenance, installer smoke checks, draft-release re-download validation, publication, and published-release re-download validation.

## Known boundaries

- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Workspace storage must be exposed to MCP before Codex handoff; workspace creation itself does not implicitly expose a storage.
- Local Filesystem storage used for workspace namespace binding must use an explicit absolute host path.
- Agent Task publication deliberately has no overwrite mode.
- Existing local symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- v0.8.0 remains the current stable release while rc.3 is evaluated. Stable public identity must not move to v0.8.1 until a stable release is separately prepared and published.
