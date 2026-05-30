# Feature and Documentation Audit — 2026-05-30

This note tracks what Infimount currently has in public `v0.7.0`, whether the main public surfaces describe it correctly, and what was fixed after the audit.

## Summary

Overall status: **aligned after follow-up polish**.

The README, GitHub Pages landing page, release notes, backend capability matrix, security model, MCP setup guide, `llms.txt`, product docs, and agent guide now describe the core `v0.7.0` product accurately: local-first desktop storage browsing, OpenDAL-backed backends, explicit MCP exposure, safe agent operations, validation clarity, Workbench flows, Agent Workspaces, and release/install flows.

## Current feature inventory and doc coverage

| Area | What we have now | README | Webpage | Other docs | Notes |
| --- | --- | --- | --- | --- | --- |
| Local-first app model | Config, storage registry, MCP settings, and credentials stay local; no hosted Infimount backend required. | Yes | Yes | Yes: security, product docs, llms | Correctly positioned. |
| Cross-platform desktop distribution | Linux DEB/RPM/AppImage, macOS DMG, Windows MSI/EXE. | Yes | Yes | Yes: release docs | Correct. |
| Single-command installers | `install.sh` for Linux/macOS, `install.ps1` for Windows, checksum verification, version pinning. | Yes | Yes | Yes: releasing | Correct and prominent. |
| Homebrew tap | Linux formula and macOS cask through `infimount/homebrew-infimount`. | Yes | Yes | Yes: release docs | Correct. |
| Supported storage backends | Local filesystem, S3/S3-compatible, Backblaze B2, Aliyun OSS, Tencent COS, Huawei OBS, Azure Blob, GCS, WebDAV. | Yes | Yes | Yes: backend matrix, llms, Agents | Webpage now uses visible S3/S3-compatible wording. |
| OpenDAL-first architecture | Storage operations route through OpenDAL; no provider-specific file-operation paths. | Yes | Yes | Yes: Agents, backend matrix, roadmap | Correct. |
| Capability matrix | Backend-dependent browse/read/write, presign, versions, metadata behavior. | Yes by link | Yes by link | Yes: backend matrix | Correct. |
| Storage validation clarity | Grouped capabilities, sanitized details, fix hints, MCP readiness warnings, copyable summaries. | Yes | Yes | Yes: release notes, backend matrix, security | Correct. |
| Secret masking | Storage-management outputs and exports mask secrets; validation avoids raw config/secrets. | Partial | Not prominent | Yes: security, release notes | Correct in security docs. Public surfaces do not need every masking detail. |
| File browser | Grid/list views, preview, drag/drop upload, create file/folder, delete, bookmarks, recents. | Yes | Yes | Roadmap/changelog | Workbench copy now surfaces daily file-manager flows. |
| Keyboard navigation | Roving keyboard nav in file grid/table, Home/End, Enter, Space selection. | Yes | Yes | Yes: changelog/roadmap | Correct. |
| Transfer workflow | Transfer queue, progress, retry/cancel, persisted history, dry-run planning, conflict handling, dual-pane copy/move/compare/update. | Yes | Yes | Yes: roadmap/changelog/product study | Now promoted in README and webpage. |
| Global search | Opt-in local global search/indexing; stop/cancel control avoids stale responses. | Yes | Yes | Yes: release notes/roadmap | Correct. |
| Agent Workspaces | Workspace creation on storage, templates, scoped MCP policy, memory files, checkpoints, workspace audit grouping. | Yes | Yes | Yes: roadmap/product study/tests | Now promoted in README and webpage. |
| MCP transports | stdio and Streamable HTTP. | Yes | High-level | Yes: MCP setup/security | Correct. |
| MCP explicit exposure | New/imported/migrated storages default to `mcp_exposed=false`; only enabled + exposed storages are visible. | Yes | Yes | Yes: security/MCP setup/release notes | Correct and important. |
| MCP tool controls | Enabled-tool list controls discovery and execution; resources respect tool gating. | Yes | Yes high-level | Yes: security/MCP setup/changelog | Correct. |
| MCP path policy | Allow/deny prefixes, deny wins, normalized segment-aware path checks, recursive descendant enforcement. | Partial | High-level | Yes: security/changelog | Correct in security docs; README only needs summary. |
| MCP confirmations | Risky writes/deletes/presign/version-delete/copy/move can require approval; approvals are scoped, single-use, fingerprinted. | Yes | High-level | Yes: security/MCP setup | Correct. |
| MCP sessions | Scoped sessions with storage/path/read-only/TTL; visible active sessions in desktop settings. | Yes | Not prominent | Yes: security/MCP setup/changelog | Correct. |
| MCP audit | Bounded local audit log, filters, copy-visible JSON, redacted export bundles with manifest. | Yes | High-level | Yes: security/MCP setup/changelog | Correct. |
| HTTP auth hardening | Non-loopback desktop HTTP requires bearer token; headless HTTP requires token unless loopback insecure dev mode. | Yes | Not detailed | Yes: security/MCP setup | Correct. |
| Version-aware tools | `list_versions`, `read_file_version`, `delete_version` where backend/config supports versions. | Yes | Not prominent | Yes: backend matrix/MCP setup | Correct. |
| Representative MCP tool names | Landing page lists real tool names. | N/A | Yes | Yes: MCP setup | Fixed `download_link` to `generate_download_link`. |
| Release automation | Release gates, artifact smoke, checksums, SBOM, provenance, post-release validation, Homebrew checksum validation. | Partial | No | Yes: releasing/workflows/scripts | Correct; not a product feature for landing page. |
| Feature-doc drift prevention | Supported backend names, S3-compatible wording, public MCP tool name, Workbench copy, and Agent Workspaces copy are checked by automation. | Yes | Yes | Yes: scripts/workflows/releasing | Added `scripts/check-feature-docs.mjs`. |

## Fixes applied from this audit

1. Updated GitHub Pages visible copy to use **S3/S3-compatible** wording.
2. Changed the landing-page representative MCP tool from `download_link` to the real `generate_download_link` tool name.
3. Added README Workbench coverage for dual-pane copy/move/compare/update, transfer queue, conflict handling, bookmarks/recents, keyboard navigation, and global search stop.
4. Added README Agent Workspaces coverage for workspace templates, scoped MCP policy, memory files, checkpoints, and workspace audit grouping.
5. Added matching Workbench and Agent Workspaces presentation to GitHub Pages.
6. Refreshed `PRODUCT.md` user examples for the current backend set and workspace-scoped MCP use.
7. Updated `docs/product-market-study.md` review date and S3/S3-compatible wording.
8. Clarified `Agents.md` storage registry/config path wording around `storages.json`, `mcp_settings.json`, and legacy `config.json`.
9. Added automated feature-doc consistency checks to repo lint, release gates, post-release validation, and local release dry runs.

## Recommended next work

### Product roadmap options

- Patch release polish: docs and UI copy cleanup only if users report confusion.
- Testing hardening: provider-like simulator coverage for OSS/COS/OBS where practical.
- `v0.8.0` planning: SFTP/FTP readiness, versioning UI, richer WebDAV/Nextcloud guidance, and broader workbench polish.

### Automation extensions

- Extend `scripts/check-feature-docs.mjs` when new public-facing features are added.
- Add screenshot/landing-page visual regression if the GitHub Pages page starts changing more often.
