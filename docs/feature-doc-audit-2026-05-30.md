# Feature and Documentation Audit — 2026-05-30

This note tracks what Infimount currently has in public `v0.7.0`, whether the main public surfaces describe it correctly, and what should be improved next.

## Summary

Overall status: **mostly aligned**.

The README, GitHub Pages landing page, release notes, backend capability matrix, security model, MCP setup guide, `llms.txt`, and agent guide all now describe the core `v0.7.0` product accurately: local-first desktop storage browsing, OpenDAL-backed backends, explicit MCP exposure, safe agent operations, validation clarity, and release/install flows.

Remaining doc polish is mostly about **prominence and exact wording**, not missing critical release claims.

## Current feature inventory and doc coverage

| Area | What we have now | README | Webpage | Other docs | Notes |
| --- | --- | --- | --- | --- | --- |
| Local-first app model | Config, storage registry, MCP settings, and credentials stay local; no hosted Infimount backend required. | Yes | Yes | Yes: security, product docs, llms | Correctly positioned. |
| Cross-platform desktop distribution | Linux DEB/RPM/AppImage, macOS DMG, Windows MSI/EXE. | Yes | Yes | Yes: release docs | Correct. |
| Single-command installers | `install.sh` for Linux/macOS, `install.ps1` for Windows, checksum verification, version pinning. | Yes | Yes | Yes: releasing | Correct and prominent. |
| Homebrew tap | Linux formula and macOS cask through `infimount/homebrew-infimount`. | Yes | Yes | Yes: release docs | Correct. README upgrade copy could be slightly clearer per platform. |
| Supported storage backends | Local filesystem, S3/S3-compatible, Backblaze B2, Aliyun OSS, Tencent COS, Huawei OBS, Azure Blob, GCS, WebDAV. | Yes | Mostly | Yes: backend matrix, llms, Agents | Webpage should say **S3-compatible**, not only S3, in visible connector copy. |
| OpenDAL-first architecture | Storage operations route through OpenDAL; no provider-specific file-operation paths. | Yes | Yes | Yes: Agents, backend matrix, roadmap | Correct. |
| Capability matrix | Backend-dependent browse/read/write, presign, versions, metadata behavior. | Yes by link | Yes by link | Yes: backend matrix | Correct. |
| Storage validation clarity | Grouped capabilities, sanitized details, fix hints, MCP readiness warnings, copyable summaries. | Yes | Yes | Yes: release notes, backend matrix, security | Correct. |
| Secret masking | Storage-management outputs and exports mask secrets; validation avoids raw config/secrets. | Partial | Not prominent | Yes: security, release notes | Correct in security docs. Public surfaces do not need every masking detail. |
| File browser | Grid/list views, preview, drag/drop upload, create file/folder, delete, bookmarks, recents. | Mostly | High-level | Roadmap/changelog | README mentions core workbench; webpage could make daily workbench features more explicit. |
| Keyboard navigation | Roving keyboard nav in file grid/table, Home/End, Enter, Space selection. | Yes in roadmap | No/implicit | Yes: changelog/roadmap | Accurate but not prominent on landing page. |
| Transfer workflow | Transfer queue, progress, retry/cancel, persisted history, dry-run planning, conflict handling, dual-pane copy/move/compare/update. | Partial | Not prominent | Yes: roadmap/changelog/product study | Consider adding a concise “Workbench” section to README/webpage. |
| Global search | Opt-in local global search/indexing; stop/cancel control avoids stale responses. | Yes | Yes high-level | Yes: release notes/roadmap | Correct. |
| Agent Workspaces | Workspace creation on storage, templates, scoped MCP policy, memory files, checkpoints, workspace audit grouping. | Partial | Not prominent | Yes: roadmap/product study/tests | Important feature is under-mentioned publicly. |
| MCP transports | stdio and Streamable HTTP. | Yes | High-level | Yes: MCP setup/security | Correct. |
| MCP explicit exposure | New/imported/migrated storages default to `mcp_exposed=false`; only enabled + exposed storages are visible. | Yes | Yes | Yes: security/MCP setup/release notes | Correct and important. |
| MCP tool controls | Enabled-tool list controls discovery and execution; resources respect tool gating. | Yes | Yes high-level | Yes: security/MCP setup/changelog | Correct. |
| MCP path policy | Allow/deny prefixes, deny wins, normalized segment-aware path checks, recursive descendant enforcement. | Partial | High-level | Yes: security/changelog | Correct in security docs; README only needs summary. |
| MCP confirmations | Risky writes/deletes/presign/version-delete/copy/move can require approval; approvals are scoped, single-use, fingerprinted. | Yes | High-level | Yes: security/MCP setup | Correct. |
| MCP sessions | Scoped sessions with storage/path/read-only/TTL; visible active sessions in desktop settings. | Yes | Not prominent | Yes: security/MCP setup/changelog | Correct. |
| MCP audit | Bounded local audit log, filters, copy-visible JSON, redacted export bundles with manifest. | Yes | High-level | Yes: security/MCP setup/changelog | Correct. |
| HTTP auth hardening | Non-loopback desktop HTTP requires bearer token; headless HTTP requires token unless loopback insecure dev mode. | Yes | Not detailed | Yes: security/MCP setup | Correct. |
| Version-aware tools | `list_versions`, `read_file_version`, `delete_version` where backend/config supports versions. | Yes | Not prominent | Yes: backend matrix/MCP setup | Correct. |
| Release automation | Release gates, artifact smoke, checksums, SBOM, provenance, post-release validation, Homebrew checksum validation. | Partial | No | Yes: releasing/workflows/scripts | Correct; not a product feature for landing page. |

## Documentation gaps to fix

1. **Webpage should say “S3-compatible” in visible copy.**
   The README and docs are correct, but the landing page visible connector copy often says only “S3”.

2. **Webpage MCP tool list uses `download_link`; the actual tool is `generate_download_link`.**
   Either rename the visible item to `generate_download_link` or describe it as “download links” without implying an exact tool name.

3. **Workbench features are under-marketed.**
   Transfer queue, dual-pane copy/move, compare/update, conflict handling, persisted transfer history, and keyboard navigation are real shipped features but are mostly buried in roadmap/changelog.

4. **Agent Workspaces are under-mentioned publicly.**
   They are documented in the roadmap/product docs but not clearly surfaced in README/webpage.

5. **`PRODUCT.md` primary-user examples are stale.**
   They mention local/S3/Azure/GCS/WebDAV but should include B2, OSS, COS, OBS, and safe MCP workflows.

6. **`docs/product-market-study.md` review date is stale.**
   It contains v0.7.0 content but still says last reviewed 2026-05-24.

7. **`Agents.md` should clarify config file responsibilities.**
   It still describes core config around `~/.infimount/config.json`; current desktop/MCP storage registry behavior centers on `~/.infimount/storages.json` plus `~/.infimount/mcp_settings.json`, with legacy migration paths.

## Recommended next work

### Small doc polish, low risk

- Update webpage visible copy to say S3/S3-compatible.
- Fix the MCP tool label from `download_link` to `generate_download_link` or generic “download links”.
- Refresh `PRODUCT.md` user examples and `docs/product-market-study.md` review date.
- Clarify `Agents.md` config/registry wording.

### Better public presentation

- Add a compact **Workbench** block to README and GitHub Pages:
  - dual-pane copy/move
  - transfer queue and retry/cancel
  - conflict handling
  - bookmarks/recents
  - keyboard navigation
  - global search with stop control
- Add a compact **Safe Agent Workspaces** block:
  - workspace templates
  - scoped MCP policy
  - memory files
  - checkpoints
  - workspace audit grouping

### Automation to prevent future drift

- Add a feature-doc consistency check that compares supported backend schema names against README, `docs/index.html`, `docs/backend-capabilities.md`, `docs/llms.txt`, and `Agents.md`.
- Extend release consistency checks to verify key public tool names such as `generate_download_link` are not misnamed on the webpage.
- Add a lightweight docs audit checklist to `docs/releasing.md` before each public tag.

### Product roadmap options

- Patch release polish: documentation cleanup plus small UI copy improvements.
- Testing hardening: more automated docs/feature drift checks and provider-like simulator coverage.
- `v0.8.0` planning: SFTP/FTP readiness, versioning UI, richer WebDAV/Nextcloud guidance, and broader workbench polish.
