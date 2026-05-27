# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Started v0.7 validation clarity work with grouped storage capability summaries, sanitized fix hints, MCP readiness notes, and copyable validation summaries in Add/Edit Storage.

### Changed

- `validate_storage` results now include versioning capability fields, fix hints, and advisory warnings without exposing storage secrets.

## [0.6.0] - 2026-05-27

### Added

- MCP Settings now includes guided access presets for read-only research, workspace agents, manual approval mode, and MCP lockdown.
- Added a policy-aware "What the agent can access" summary covering exposed storages, enabled tools, write/destructive/link access, confirmations, active sessions, and recent risky actions.
- Added MCP audit filtering by text, decision, and storage, plus copy-visible JSON and export-visible local audit bundles with redaction manifests.
- Added active scoped MCP session visibility in desktop MCP Settings by reusing the desktop HTTP runtime session manager.
- Added roving keyboard navigation for virtualized file grid and table views, including arrow-key movement, Home/End jumps, Enter open, and Space toggle selection.
- Added MCP safety scenario coverage for allowed reads, denied prefix escape attempts, read-only session write blocking, confirmation replay protection, cross-storage copy from read-only sources, and audit redaction behavior.

### Changed

- Presets save enabled tools and update policies only for storages already exposed to MCP, preserving explicit storage exposure.
- Desktop HTTP now requires a bearer token for non-loopback bind addresses; unauthenticated HTTP is limited to loopback local development.
- MCP resources now respect enabled-tool controls so disabled read/list/stat tools cannot be bypassed through resource APIs.
- Recursive list, search, copy, delete, and overwrite flows enforce MCP path policy for descendant paths so denied child prefixes cannot be exposed or mutated through an allowed parent.
- Confirmation checks now validate session scope before creating pending approvals, so read-only sessions receive deterministic denial instead of approval prompts.
- Cross-storage copy confirmation checks treat the source as read-like and the destination as write-like.
- Updated agent-facing architecture docs to reflect native Backblaze B2 and the current OpenDAL-backed backend set.

### Fixed

- Fixed desktop Tauri storage draft validation so native Backblaze B2 is accepted by add, update, and verify flows.

## [0.5.0] - 2026-05-26

### Added

- Native Backblaze B2 support across core, desktop, and MCP using OpenDAL.
- S3 `defaultAcl` configuration for buckets that require a default object ACL.
- WebDAV `disableCreateDir` compatibility mode for servers that reject collection creation probes/placeholders.
- Capability reporting and capability-gated writes for OpenDAL user metadata.
- MCP `stat_path` now returns user metadata when OpenDAL exposes it.

### Changed

- Upgraded OpenDAL to 0.56.0, migrated recursive delete calls to `delete_with(...).recursive(true)`, and documented Rust 1.85+ as the minimum Rust version for source builds.
- Clarified the public roadmap so v0.4.0 contains the completed Workbench Reliability and Agent Workspaces work, while public v0.5.0 focuses on backend expansion.
- Documented that Volcengine TOS was assessed but is not exposed until OpenDAL reports product-ready read/write/list/stat capability.

## [0.4.0] - 2026-05-26

### Added

- Single-command install scripts for macOS/Linux and Windows with release checksum verification.
- Transfer queue panel for copy/move work with queued/running/completed/failed states, retry, active or queued cancellation, and progress visibility backed by Tauri transfer progress events.
- Split-pane browsing with an independently selectable destination pane and direct copy/move actions between panes.
- Conflict resolution now supports keeping both items by auto-renaming the incoming transfer.
- Folder bookmarks, recent folders, provider presets, and persisted transfer history make repeated workbench flows faster.
- Rust coverage gate increased from 50% to 54% lines.
- Documented the OpenDAL-first storage policy: future file operations should remain backend-agnostic rather than adding provider-specific SDK paths.
- Added the internal workbench roadmap and started transfer reliability with an OpenDAL-only transfer dry-run manifest API.
- Added workbench foundations: recursive metadata scans, opt-in local global search indexing, transfer activity log events, dry-run summaries in the transfer queue, interrupted-transfer recovery behavior, and dual-pane compare/update flows.
- Completed Agent Workspaces foundation with OpenDAL-backed workspace creation, scoped MCP policy application, coding/research/data-analysis templates, visible memory files, OpenDAL-written checkpoint manifests, checkpoint restore, and grouped workspace audit activity.

## [0.2.3] - 2026-05-14

### Added

- MCP client setup documentation with stdio and HTTP examples.
- Security documentation covering local config storage, secret masking, MCP HTTP auth, and session scoping.
- Backend capability matrix for versioning, presign, copy, rename, and metadata behavior across supported storage backends.
- Product and design reference documents to keep future UI work aligned with Infimount's local-first, native-file-manager direction.
- Public release link checker script for validating stable GitHub Release asset URLs before announcements.

### Changed

- Improved MCP HTTP runtime hardening with stricter auth-token normalization and safer handling of missing or whitespace-only tokens.
- Reused the MCP session manager across HTTP sessions so scoped access remains consistent during a running server lifetime.
- Hid disabled MCP tools from tool discovery and rejected disabled tool calls consistently.
- Reworked MCP settings UI for clearer runtime status, tool-level exposure controls, app-native confirmation dialogs, and better contrast.
- Replaced browser-native confirmation prompts with app-native dialogs for update install, non-loopback MCP HTTP startup, and version deletion.
- Updated the GitHub Pages landing page with a more polished product presentation, download sections, MCP messaging, SEO metadata, and install notes.
- Updated README download, MCP, storage capability, and security references.
- Lazy-loaded file icon theme packs to reduce startup bundle pressure while preserving selectable icon themes.

### Fixed

- Removed double focus highlights from Add Storage and MCP settings input fields.
- Kept Add Storage validation available from the bottom action row while allowing clicks to surface inline required-field errors.
- Improved sidebar/update dialog behavior by avoiding browser-native prompts.
- Added workflow compatibility environment settings for newer GitHub-hosted JavaScript action runtimes.

## [0.1.0] - 2026-03-01

First stable release of Infimount — a unified desktop storage browser powered by Apache OpenDAL.

### Features

- **Unified file browser** for local filesystem, Amazon S3, Azure Blob Storage, Google Cloud Storage, and WebDAV
- **Grid and list views** with smooth transitions
- **File preview panel** with inline image, text, and document rendering
- **Drag-and-drop uploads** into any storage backend
- **Drag-select** for multi-file operations
- **Create folder and file** from the UI
- **Storage sidebar** with multiple source management and reordering
- **Verify storage connection** button for validating backend credentials
- **Cross-platform desktop app** — native builds for Linux (.deb, .rpm, .AppImage), macOS (.dmg), and Windows (.msi, .exe)
- **Custom window chrome** with transparent titlebar on macOS
- **System tray** integration with quit menu

### Architecture

- **Rust core** (`infimount_core`) with OpenDAL for backend-agnostic storage operations
- **Tauri 2** bridge layer connecting React frontend to Rust backend
- **React 19 + TypeScript** frontend with Radix UI components
- **IndexMap-based** source registry preserving insertion order (newest-first)
- **Local-first config** — credentials stored on your machine, no cloud dependency

### Infrastructure

- GitHub Actions **CI pipeline** with `cargo fmt`, `clippy`, `cargo test`, ESLint, and TypeScript checks
- **Multi-platform release workflow** with smoke tests, SHA256 checksums, SBOM (SPDX), and build provenance attestation
- Optional **macOS code signing** and **Windows code signing** when secrets are configured
- **Dependabot** for automated dependency updates (Cargo, npm, GitHub Actions)
- **Governance documentation** — GOVERNANCE.md, MAINTAINERS.md, CODEOWNERS, SECURITY.md
- **Homebrew tap** available at `infimount/infimount`

---

[Unreleased]: https://github.com/infimount/infimount/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/infimount/infimount/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/infimount/infimount/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/infimount/infimount/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/infimount/infimount/releases/tag/v0.3.0
[0.2.3]: https://github.com/infimount/infimount/compare/v0.1.0...v0.2.3
[0.1.0]: https://github.com/infimount/infimount/releases/tag/v0.1.0
