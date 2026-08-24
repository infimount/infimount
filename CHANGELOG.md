# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0] - Unreleased release candidate

### Added

- Added native OS secret store integration with automatic plaintext migration for stored credentials.
- Added shareable storage export/import with config preview, conflict resolution, and backup/restore.
- Added passphrase-protected recovery backup with age encryption, integrity verification, and decryption preview.
- Added workspace profiles with file-locked atomic persistence and browser-storage migration.
- Added multi-step activation wizard covering storage, MCP, client, and verification.
- Added diagnostics bundle export with privacy-safe redaction, checksums, and leak validation.
- Added structured product event log with local JSONL storage, rotation, and opt-in telemetry consent.
- Added revision-keyed operator caches for desktop and MCP storage access, with invalidation after configuration or secret changes.
- Added paginated directory listing with cursor support for non-recursive and recursive scans.
- Added ranged file reads with configurable max (256 KiB default, 2 MiB absolute).
- Added guided desktop OAuth connection for Google Drive and Microsoft OneDrive using browser authorization, local loopback callback, PKCE S256, state validation, and local-only token storage.
- Added optional credential-gated OAuth storage smoke coverage and setup documentation for maintainers.

### Changed

- Set `withGlobalTauri` to `false` with restrictive CSP for hardened security.
- Replaced flat onboarding dialog with multi-step ActivationWizard.
- Migrated workspace storage from `localStorage` to Tauri-backed `WorkspaceRegistry`.
- Bound workspaces to the storage namespace identity (fingerprint of backend, account, container, and root) and verified the binding whenever workspace access or policy rules change.
- Rejected storage namespace edits and storage removal while workspaces remain bound, and required explicit confirmation plus validation before committing storage credential changes that affect bound workspaces.
- Disabled pre-v0.8 Agent Workspaces migration: the legacy import command is unregistered and the browser storage keys are never read or removed.
- Bounded version listing across desktop and MCP to a single shared implementation: at most 10,000 versions are scanned per path and 1,000 returned per page, with an HMAC-signed cursor bound to the storage, path, and storage revision.
- Staged desktop uploads under a per-user owned staging directory (`$XDG_RUNTIME_DIR/infimount/upload-staging` or the per-user cache directory) instead of a shared predictable path, with a cross-process lock and verified stale-cleanup that never follows symlinks.
- Bound MCP `write_file` to 4 MiB and required atomic create-if-absent semantics for no-overwrite writes; backends that cannot guarantee atomic no-overwrite now return an explicit unsupported-backend error instead of a stat-then-write race.
- Made directory transfers transactional: new destinations are staged and committed with a rename where the backend supports it, and failed or cancelled transfers remove the transaction-created destination while never deleting a destination that existed beforehand.
- Hardened MCP client command integration: installed client CLIs are captured by identity (canonical path and digest), run under a 30-second timeout with process-tree cleanup, roll back configuration on failure, and never surface raw command output in errors.
- Made activation reporting explicit: the wizard now reports scope-isolation and safe-default-profile outcomes and ties activation probe events to the current probe run in the MCP audit log.
- Added a local product-event persistence toggle that is independent of network telemetry consent; disabling it stops local event writes without changing network export behavior.

### Security

- Reinforced OAuth secret handling so access tokens, refresh tokens, client secrets, authorization codes, device codes, and PKCE verifiers remain masked from UI text, validation output, logs, audit exports, and token-exchange errors.
- Preserved explicit MCP opt-in: connecting Google Drive or OneDrive does not expose the storage to MCP clients by default.
- Rejected copy/move transfers that would nest a storage into its own or a peer storage's subtree, and rejected manual storage-policy edits that would override workspace-managed rules.
- Removed unused file-based backup helpers (`encrypt_backup_to_file`/`decrypt_backup_from_file`) that were not reachable from production paths.
- Aligned the embedded updater public key with the signing identity that has produced every release since v0.7.1; the previously embedded key was orphaned (its private half never existed), so the v0.7.x auto-update path never verified. v0.7.x users must install v0.8 manually once.
- Updater signature verification now accepts both classic minisign text and Tauri's base64-wrapped `.sig` format before cryptographic verification.

### Fixed

- Startup recovery now runs under one cross-process configuration transaction lock in a fixed order — restore recovery, import transactions, secret-reference transactions, secret cleanup, then plaintext/legacy migration cleanup — so credential cleanup can never delete accounts a pending transaction still needs. A failed interrupted-restore recovery blocks all later cleanup until the next launch.
- The bundled MCP sidecar refuses configuration cleanup while a desktop restore journal is pending instead of racing desktop recovery.

## [0.7.1] - 2026-06-05

### Added

- Added OpenDAL-backed SFTP and FTP storage backends across desktop schemas, Rust core builders, MCP storage management, capability docs, and tests.
- Added SFTP configuration for endpoint, username, private key path, remote root, known-hosts strategy, and optional remote-copy extension use.
- Added FTP configuration for endpoint, username, password, and remote root.
- Added agent integration examples for Claude Desktop, generic HTTP MCP clients, OpenCode, and a Pi extension wrapper.

### Changed

- Reworked split-pane browsing into a same-storage, same-folder native file-manager mode with a shared header, left/right pane labels, and a visible close control.
- Consolidated OpenDAL operator construction and shared filesystem operations through `infimount_core` so desktop and MCP behavior stay aligned.
- Centralized MCP recursive list/copy/move/delete flows around core operations while preserving explicit MCP storage policy, session, read-only, and confirmation checks.
- Strengthened release automation with post-release validation, release consistency checks, install-script smoke checks, feature-doc drift checks, and zero-manual release gate policy checks.

### Fixed

- Fixed recursive folder transfer safety by blocking self-descendant copies such as copying `/demo` into `/demo/child`.
- Fixed transfer planning feedback, cancellation during planning, fallback destination checks, and sanitized storage errors so backend URLs and query strings are not exposed in UI messages.
- Fixed delete UX by showing visible delete progress and requiring confirmation for visible single-file delete actions.
- Fixed upload UX so progress is tied to actual writes, cancellation stops remaining uploads, and existing-name conflicts require an explicit skip/keep-both/overwrite decision.
- Fixed destructive core mutation guards by refusing storage-root delete, ignoring current-directory markers during recursive traversal, and rejecting duplicate batch destinations before mutation.
- Fixed unsupported backend handling so unknown backends are rejected explicitly instead of falling back to local storage.
- Fixed macOS/app icon corner rounding.

### Tests

- Added Playwright snapshot coverage for split-pane same-storage browsing and delete progress.
- Added regression coverage for upload progress/conflict handling, transfer cancellation, recursive transfer semantics, root delete refusal, duplicate transfer destinations, MCP policy-aware recursion, and secret masking for SFTP private key paths.

## [0.7.0] - 2026-05-30

### Added

- Added OpenDAL-backed storage support for Aliyun OSS, Tencent COS, and Huawei OBS across desktop schemas, Rust core builders, MCP builders, capability docs, and tests.
- Added capability-aware storage validation with grouped browse/mutation/sharing summaries, sanitized fix hints, MCP readiness notes, and copyable validation summaries in Add/Edit Storage.
- Added a stop control for global search indexing so stale in-flight recursive list responses are ignored after cancellation or dialog close.

### Changed

- New storage additions, imports, and legacy migrations now default to not exposed to MCP, preserving explicit agent-access opt-in.
- `validate_storage` results now include versioning capability fields, fix hints, and advisory warnings without exposing storage secrets.
- Storage-management tools canonicalize backend aliases such as `aliyun_oss`, `tencent_cos`, and `huawei_obs` before persistence.
- Refined the GitHub Pages install section and README presentation with clearer Linux, macOS, and Windows install paths, copy buttons, and mobile-friendlier download links.

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

[Unreleased]: https://github.com/infimount/infimount/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/infimount/infimount/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/infimount/infimount/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/infimount/infimount/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/infimount/infimount/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/infimount/infimount/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/infimount/infimount/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/infimount/infimount/releases/tag/v0.3.0
[0.2.3]: https://github.com/infimount/infimount/compare/v0.1.0...v0.2.3
[0.1.0]: https://github.com/infimount/infimount/releases/tag/v0.1.0


### Post-merge RC hardening

- Reject symlink and Windows reparse-point components for local MCP paths.
- Block new configuration mutations until pending import, restore, secret, and
  cleanup transactions are recovered.
- Make built-in MCP HTTP loopback-only and keep environment tokens headless-only.
- Remove obsolete credential references after successful recovery restore.
- Fail closed when deleting a workspace whose managed policy grant is missing.
- Extend advanced credential-name detection and add safe download commit fallback.
- Move GitHub Actions validation to Node.js 24.

### Final transaction and HTTP corrections

- Hold one configuration transaction across workspace creation and storage-import
  workspace validation/commit.
- Separate restore commit from credential cleanup and persist obsolete references
  in the committed restore journal for deterministic startup retry.
- Preserve active secret-cleanup entries until their persisted references are
  removed.
- Parse loopback bind addresses as IP addresses instead of accepting arbitrary
  hostnames beginning with `127.`.
