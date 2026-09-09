# Infimount 0.8.1-rc.2: Agent Tasks pilot hardening

Release: not published yet.

Infimount 0.8.1-rc.2 is the second release candidate for Agent Tasks. It keeps the Agent Tasks safety model from rc.1 and fixes the material workspace-creation and installation-discovery issues found while starting the first real v0.8.0-to-candidate pilot.

## Why rc.2

The first real rc.1 pilot did not reach Agent Task execution. During candidate setup, Agent Workspace creation exposed two workflow problems:

- the creation dialog still treated a workspace as a coding/research/data-analysis agent template instead of primarily as a scoped storage/MCP boundary;
- a Local Filesystem storage using shell-style path notation could be saved but fail later during workspace namespace binding with an absolute-root error.

The pilot was therefore classified as an Infimount workflow defect and rc.1 is not treated as completed product-validation evidence. rc.2 fixes the blocker before the pilot resumes.

## Agent Workspace creation

### Simpler boundary-first model

- New Agent Workspaces are plain scoped folders. New creation no longer asks the user to choose a coding, research, or data-analysis agent type.
- The primary creation flow is now workspace name, storage, derived storage-relative workspace location, and read/write choice.
- The workspace folder is derived under `/agent-workspaces/<workspace-name>` and shown before creation instead of asking for a second path.
- The managed workspace MCP rule is part of normal workspace creation and is no longer an optional creation toggle.
- Read-only remains the safe default. Agent writes remain an explicit desktop opt-in and are required for Agent Tasks that produce outputs.
- Existing current-schema templated workspaces remain supported. Their existing starter-note and checkpoint files are preserved as compatibility behavior, but templates are not part of new workspace identity.

### Earlier validation for Local Filesystem roots

- Workspace creation now checks Local Filesystem storage roots before mutation and surfaces an actionable storage-level error instead of failing deep in namespace fingerprinting.
- Workspace namespace binding requires an explicit absolute host filesystem root. Shell notation such as `$HOME/...`, tilde paths such as `~/...`, missing roots, and relative roots are rejected for workspace use.
- The path shown for the workspace itself remains storage-relative and is not another host filesystem root.

## Installer clarity

- Linux installation now detects when another `infimount` executable earlier in `PATH` shadows the executable that was just installed and prints an explicit warning with both paths.
- This addresses the pilot setup case where the v0.8.0 Debian package installed correctly while an older Linuxbrew v0.2.2 executable continued to launch from the shell.
- The public Homebrew tap remains independently versioned; the warning is about local executable precedence, not a replacement for package-manager upgrades.

## Agent Tasks safety model

rc.2 does not broaden the Agent Tasks authorization surface:

- task preparation copies only explicitly selected source files and never mutates or newly exposes the source storage;
- Codex handoff uses the existing task/workspace MCP boundary;
- output review captures fresh size and SHA-256 fingerprints;
- publication starts with nothing selected and requires explicit output, destination, and conflict-policy selection;
- publication supports only `fail` or `rename`, has no overwrite mode, uses create-only writes, and verifies committed destination bytes;
- stale publication previews are rejected;
- fully successful publications write unique create-only `publish-receipt-<publication-id>.json` receipts;
- partial multi-file failures surface cleanup-required state instead of claiming atomic rollback.

See `docs/agent-tasks.md` for the complete contract and `docs/agent-workspaces.md` for the revised workspace model.

## Product validation

`v0.8.1-rc.2` supersedes rc.1 as the candidate for the real Agent Tasks pilot. The pilot should restart from an installed v0.8.0 environment and exercise:

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

The release remains subject to the normal zero-manual automated release gates: frontend lint/typecheck/unit/integration/coverage, Playwright UI tests, Rust format/clippy/tests/coverage, desktop smoke, OpenDAL simulator tests, dependency audit, release consistency, signing policy, checksums, SBOM, provenance, updater-signature verification, artifact smoke checks, and post-publication re-download validation.

## Known boundaries

- Agent Tasks v1 still require a read-write Local Filesystem Agent Workspace.
- The workspace storage must be exposed to MCP before Codex handoff; workspace creation itself does not implicitly expose a storage.
- Local Filesystem storage used for workspace namespace binding must use an explicit absolute host path.
- Agent Task publication deliberately has no overwrite mode.
- Existing local symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- v0.8.0 remains the current stable release while rc.2 is evaluated. The stable public identity must not move to v0.8.1 until a stable release is separately prepared and published.
