# Infimount 0.8.1-rc.1: Agent Tasks

Release: not published yet.

Infimount 0.8.1-rc.1 is the first release candidate for Agent Tasks, a bounded workflow for preparing selected storage files for an existing coding agent, reviewing the resulting outputs, and publishing only explicitly approved unchanged bytes.

## What is new

### Agent Tasks

- Prepare only explicitly selected source files into a task package inside an existing read-write Local Filesystem Agent Workspace.
- Keep the source storage unchanged and do not grant new MCP access to it during task preparation.
- Launch prepared tasks in Codex through the existing Infimount MCP integration rather than a separate storage-access path.
- Discover and review files under `outputs/` with byte size, SHA-256, and bounded text previews.
- Start publication with nothing selected. The desktop user must explicitly select outputs, destination storage, destination folder, and conflict handling.
- Require an exact publication preview before apply. Any selection, destination, conflict-policy, output-byte, or relevant storage-state drift invalidates the reviewed plan.
- Publish with create-only writes. Agent Task publication supports explicit `fail` or `rename` conflict handling and has no overwrite mode.
- Re-hash source outputs before and during copy, then verify committed destination bytes after close.
- Write a unique create-only `publish-receipt-<publication-id>.json` after a fully successful publication.
- Surface partial multi-file publication failures as cleanup-required when earlier objects may already have committed instead of pretending the operation was atomic.

### Release and validation hardening

- Agent Tasks remain on the planned v0.8.1 pre-1.0 release line.
- Release consistency and stable-document promotion are version-agnostic rather than hard-coded to a previous release transition.
- Hermetic release rehearsal derives the next patch `rc.1` from a stable checkout and rehearses an already-versioned prerelease verbatim.
- Release tooling is explicitly prevented from inferring or fabricating real-world pilot completion.
- The review-before-publish Agent Task flow has action-driven Playwright coverage and a committed CI-produced visual baseline.

## Safety model

The release candidate preserves the same Infimount authorization boundary:

- Agent Task metadata is guidance and snapshot metadata, not authorization.
- Existing storage and workspace MCP policy remains authoritative.
- Agent Task publication is a desktop-only action and is not exposed as an Agent Task MCP administration tool.
- Local filesystem operations retain the existing symlink and Windows reparse-point defenses and their documented check-then-OpenDAL TOCTOU limitation.
- Storage I/O continues through the existing OpenDAL-backed abstraction.

See `docs/agent-tasks.md` for the complete contract, limits, concurrency model, and deliberate non-goals.

## Product validation

This release candidate is intended to support real product validation before broader promotion. The next phase should exercise representative coding, document, and data-analysis tasks plus the v0.8.0 to v0.8.1 install/update path.

Pilot evidence is not a manual product-test requirement in the automated release gate. The release workflow continues to rely on automated frontend, Playwright, Rust, desktop-smoke, storage-simulator, dependency-audit, coverage, consistency, and artifact/signing gates.

## Known boundaries

- The initial Agent Task workspace must be a read-write Local Filesystem Agent Workspace.
- Agent Task publication deliberately has no overwrite mode.
- The first implementation is bounded to 1,000 explicit source selections, 10,000 prepared files, 2 GiB of prepared input bytes, 100 requested outputs, 100 selected publication outputs, and 2 GiB of selected publication bytes.
- Eliminating hostile local-filesystem TOCTOU races requires a future handle-relative or `openat2`-style local backend.

## Upgrade note

v0.8.0 remains the current stable release while this candidate is evaluated. Release manifests are derived from the exact prerelease tag by the release workflow. Do not change the public stable-release identity to v0.8.1 until a stable release is actually prepared and published.
