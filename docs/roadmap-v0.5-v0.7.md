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

## Released in public v0.5.0: Backend Expansion

Theme: broaden storage coverage and compatibility without leaving the OpenDAL abstraction.

Delivered in v0.5.0:

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

## Future public v0.6.0 candidates

- SFTP, if OpenDAL support meets the product-ready rule.
- FTP, only with clear security warnings and capability coverage.
- OSS, COS, OBS, and similar object stores through OpenDAL.
- Additional WebDAV presets and compatibility toggles.
- Additional S3-compatible presets where they materially reduce setup friction.
- TOS once OpenDAL exposes useful read/write/list/stat capability.

## In progress for public v0.6.0: Safe Agent Operations

Theme: make safe MCP access easier to understand, apply, and audit.

Delivered so far:

- MCP Settings includes a policy-aware "What the agent can access" summary.
- Access presets for read-only research, workspace agents, manual approval mode, and MCP lockdown, with guided copy for common agent/workspace uses.
- Presets save enabled tools and exposed storage path policies without exposing hidden storages.
- MCP audit filtering by text, decision, and storage.
- Copy-visible audit export for redacted local audit review.
- Export-visible audit bundles under `~/.infimount/exports/` with a redaction manifest.
- Active scoped session visibility in MCP Settings, backed by the same session manager used by the desktop HTTP runtime.
- MCP safety scenario tests for allowed reads, denied prefix escape attempts, write confirmations, confirmation replay protection, read-only session write blocking, and audit redaction.

v0.6.0 Safe Agent Operations is now implementation-complete for the planned public scope. Remaining hardening moves into v0.6.1 workbench polish.

## In progress for public v0.6.1: Workbench Polish

Theme: make daily file-manager work faster, more accessible, and more predictable under large or interrupted workloads.

Delivered so far:

- Roving keyboard navigation in the virtualized file grid and table views.
- Arrow keys move between files and folders, Home/End jump to the first or last entry, Enter opens the focused item, and Space toggles selection.
- File items now expose selection state and visible focus rings for keyboard users.

Next candidates:

- Keyboard navigation hardening across the sidebar, dialogs, and MCP settings.
- Cancel in-flight list/search work so slow storage responses do not keep the workbench feeling blocked.
- Better storage validation UX with capability summaries and fix hints.

## Future quality work

- Continue frontend branch/function coverage hardening for `FileBrowser.tsx`, `FilePreviewPanel.tsx`, and `McpSettingsDialog.tsx`.
- Keep release artifact builds blocked behind the automated zero-manual release gate.
- Keep storage-simulator coverage focused on OpenDAL behavior and safe fallbacks instead of provider-specific SDK paths.
