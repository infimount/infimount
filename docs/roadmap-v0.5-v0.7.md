# Roadmap: public v0.5 and beyond

Infimount remains OpenDAL-first. File operations route through OpenDAL-backed Rust core/MCP layers, not provider-specific SDK paths.

## Released in public v0.4.0: Workbench Reliability and Agent Workspaces

The internal workbench and agent-workspace milestones were bundled into public `v0.4.0` so the public roadmap can move forward with fewer confusing internal version labels.

Delivered workbench reliability:

- Transfer queue and persisted transfer history.
- OpenDAL-only transfer dry-run planning with `plan_transfer_entries`.
- Dry-run summaries and local activity-log events for planned, started, completed, failed, cancelled, and recovery-started transfers.
- Interrupted-transfer recovery that restores failed/retryable jobs and skips completed files when safe.
- Recursive OpenDAL metadata scans for opt-in local path/name/metadata indexing.
- Global search across indexed storages from the storage menu.
- Dual-pane compare that detects missing/changed files and copies/updates through the transfer queue.

Delivered Agent Workspaces:

- Workspace creation on OpenDAL-backed storage.
- Workspace-scoped MCP policy using default-deny access and a workspace-root allowlist.
- Coding, research, and data-analysis templates.
- Visible memory files under `memory/`.
- OpenDAL-written workspace/checkpoint manifests under `.infimount/checkpoints`.
- Checkpoint restore for visible memory files.
- Workspace activity grouping across local events and MCP audit events that fall under the workspace root.

## Active public v0.5.0: Backend Expansion

Theme: broaden storage coverage and compatibility without leaving the OpenDAL abstraction.

Implemented so far:

- Native Backblaze B2 backend across core, desktop, and MCP.
- B2 schema, add-storage UI mapping, operator builders, capability tests, and docs.
- S3 `defaultAcl` pass-through for buckets that require a default object ACL.
- WebDAV `disableCreateDir` compatibility mode for servers that reject collection creation probes/placeholders.
- `write_with_user_metadata` capability reporting in validation/capability payloads.
- Capability-gated user metadata writes for desktop/API and MCP `write_file`; unsupported backends return an explicit error instead of silently dropping requested metadata.
- `stat_path` returns user metadata when OpenDAL exposes it.
- Volcengine TOS assessed but not exposed because the current OpenDAL service does not report product-ready read/write/list/stat capability.

Rule for every new backend or backend-specific option:

```text
new backend = OpenDAL operator + capability matrix + tests + docs
```

No direct provider SDKs for file operations.

## Future backend candidates

- SFTP, if OpenDAL support meets the product-ready rule.
- FTP, only with clear security warnings and capability coverage.
- OSS, COS, OBS, and similar object stores through OpenDAL.
- Additional WebDAV presets and compatibility toggles.
- Additional S3-compatible presets where they materially reduce setup friction.
- TOS once OpenDAL exposes useful read/write/list/stat capability.

## Future quality work

- Continue frontend branch/function coverage hardening for `FileBrowser.tsx`, `FilePreviewPanel.tsx`, and `McpSettingsDialog.tsx`.
- Keep release artifact builds blocked behind the automated zero-manual release gate.
- Keep storage-simulator coverage focused on OpenDAL behavior and safe fallbacks instead of provider-specific SDK paths.
