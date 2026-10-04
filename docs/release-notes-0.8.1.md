# Infimount 0.8.1: Agent Tasks and Safer Agent Workflows

> v0.8.1 is not published yet.

Release: not published yet.

Infimount 0.8.1 adds review-first Agent Tasks and completes the v0.8 agent-access model with stronger least-privilege defaults, safer workspace binding, resilient file browsing, and release/package validation that exercises the real v0.8.0 upgrade path.

## Highlights

- **Agent Tasks:** prepare only explicitly selected source files into a bounded local task workspace, hand the prepared task to Codex through Infimount's existing MCP boundary, review generated outputs, and publish only explicitly selected unchanged outputs.
- **Review before publication:** output review records byte size and SHA-256, publication starts with nothing selected, and every write requires an exact destination preview.
- **Create-only publication:** publication supports only `fail` or `rename` conflict handling, never overwrite, and verifies committed destination bytes before writing a unique receipt.
- **First-class Agent Access:** storage-only users can complete onboarding without enabling agents; Agent Access remains available later as a dedicated product surface.
- **Least privilege:** guided read-only access enables only read tools; guided writable workspaces add only the bounded write tools required by the flow.
- **Clear transport semantics:** stdio is client-launched/on-demand, while HTTP exposes explicit local start/stop state. Guided access rejects broader tool surfaces or non-loopback HTTP and sends advanced configurations to Advanced MCP settings.
- **Resilient browsing:** large directories auto-continue pagination and transparently recover when a revision-bound continuation cursor becomes stale.
- **Legacy Local Filesystem compatibility:** legacy `~` and `~/...` roots use the same canonical home-alias handling across browsing, workspace identity, MCP confinement, and Agent Task preparation; shell-variable forms such as `$HOME/...` remain invalid.
- **Scoped task MCP:** the dedicated Agent Task sidecar remains independently scoped to the prepared task/workspace and does not depend on enabling general Agent Access.
- **Current Codex compatibility:** the first-class handoff enables the Codex code-mode host router required for MCP routing while keeping code mode itself, shell/unified execution, hooks, browser/computer use, plugins, project instructions, and other ambient execution surfaces disabled.

## Security and safety

Agent Task preparation does not mutate selected source files and does not broaden source-storage MCP exposure. Task outputs remain inside the explicitly writable local Agent Workspace until the desktop user reviews and approves a publication plan.

Publication rebuilds the approved plan under storage/workspace mutation locks, rejects stale previews, re-hashes reviewed source bytes, uses create-only writes, and verifies committed destination hashes. Partial multi-file failures preserve already committed objects and report cleanup-required state instead of attempting an unsafe delete rollback.

General Agent Access fails closed when disabled. Workspace readiness requires the exact managed workspace rule, expected prefix/access mode, `default_access=none`, and no broader positive manual grant. New storages remain unexposed to MCP by default.

Storage credentials, OAuth tokens, and the desktop MCP bearer token remain in the operating system's native secret store. Current v0.8 releases do not create a new plaintext credential fallback.

## Upgrade from v0.8.0

Install v0.8.1 over the existing v0.8.0 installation. The release pipeline exercises the real Linux installer-over-install path and verifies that the installer itself does not mutate user configuration, the candidate launches successfully, and representative storage/workspace state is retained.

The v0.8.0 stable updater channel remains authoritative until v0.8.1 is published. Prerelease installer validation does not claim that prereleases are offered through the stable updater channel.

Users upgrading directly from v0.7.x should first follow [Migrating from v0.7 to v0.8](migration-v0.8.md) and retain a pre-upgrade backup.

## Release validation

The v0.8.1 release line is qualified through:

- CI, Integration Tests, Coverage, Dependency Audit, Repo Lint, and Release Rehearsal;
- the real desktop Release Pilot covering onboarding, Agent Access, workspace binding, pagination recovery, Agent Task publication safety, and scoped task MCP confinement;
- Linux, macOS, and Windows package builds;
- cryptographic updater signing and key-correspondence verification;
- checksums and per-file checksum files;
- SPDX SBOM coverage including the packaged MCP sidecar;
- GitHub artifact provenance;
- draft release upload and re-download validation;
- publication and public re-download validation;
- automatic Post Release Validation;
- a real v0.8.0-to-candidate Linux installer-over-install check before publication and again from the published package.

Real coding, document, and data-analysis Agent Task runs are retained as product-validation evidence. Automated tooling may verify deterministic correctness and safety properties, but it does not self-certify human usefulness.

## Platform signing

Every v0.8.1 release artifact uses the cryptographically signed Tauri updater chain. macOS notarization/platform signing and Windows Authenticode are included and verified when the corresponding release credentials are configured. If a platform signing identity is not configured, the release explicitly reports that platform as unsigned; users may therefore encounter Gatekeeper or SmartScreen warnings and should verify checksums, provenance, and the release signing status.

## Known limitations

- FTP operations remain disabled in v0.8 because of the documented upstream dependency security issue.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace for writable task outputs.
- Agent Task publication intentionally has no overwrite mode.
- The local-filesystem path-confinement model rejects symlink/reparse-point components but retains a documented short TOCTOU boundary against a concurrently hostile local process.
- Backend capabilities such as object versions, presigned links, rename, and server-side copy remain backend/account dependent.
- Native secret-store operations require an available, unlocked OS user session.
- Rust dependency audit currently carries one documented `RUSTSEC-2023-0071` exception on the bounded Azure signing dependency path and reports allowed maintenance/unsound/yanked warnings; the release audit keeps those visible rather than hiding them.

See [Agent Tasks](agent-tasks.md), [Agent Workspaces](agent-workspaces.md), [Security](security.md), [Migration](migration-v0.8.md), [Recovery](recovery.md), and [Troubleshooting](troubleshooting.md) for operational details.
