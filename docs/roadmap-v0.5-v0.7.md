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

## Prepared for public v0.6.0: Safe Agent Operations and Workbench Polish

Theme: make safe MCP access easier to understand, apply, and audit while improving daily file-manager keyboard work.

Delivered for v0.6.0:

- MCP Settings includes a policy-aware "What the agent can access" summary.
- Access presets for read-only research, workspace agents, manual approval mode, and MCP lockdown, with guided copy for common agent/workspace uses.
- Presets save enabled tools and exposed storage path policies without exposing hidden storages.
- MCP audit filtering by text, decision, and storage.
- Copy-visible audit export for redacted local audit review.
- Export-visible audit bundles under `~/.infimount/exports/` with a redaction manifest.
- Active scoped session visibility in MCP Settings, backed by the same session manager used by the desktop HTTP runtime.
- Desktop HTTP requires a bearer token before binding beyond loopback.
- MCP resources respect enabled-tool controls so disabled filesystem tools cannot be bypassed through resource APIs.
- Recursive list, search, copy, delete, and overwrite flows enforce MCP path policy for descendant paths so denied child prefixes cannot be exposed or mutated through an allowed parent.
- MCP safety scenario tests for allowed reads, denied prefix escape attempts, write confirmations, confirmation replay protection, read-only session write blocking, cross-storage copy from read-only sources, recursive denied descendants, and audit redaction.
- Roving keyboard navigation in the virtualized file grid and table views.
- Arrow keys move between files and folders, Home/End jump to the first or last entry, Enter opens the focused item, and Space toggles selection.
- File items now expose selection state and visible focus rings for keyboard users.

## In progress for public v0.7.0: Object Storage Expansion and Validation Clarity

Theme: broaden object-store coverage without leaving OpenDAL, and make storage setup safer and more understandable before users browse or expose a backend to MCP clients.

Delivered so far for v0.7.0:

- Aliyun OSS, Tencent COS, and Huawei OBS builders across Rust core and MCP.
- Desktop Add/Edit Storage schemas for OSS, COS, and OBS with secret-aware credential fields.
- Capability and builder tests for OSS, COS, and OBS.
- Secret masking hardening for camelCase and snake_case access key IDs, application keys, and service-account credentials.
- Desktop Add/Edit Storage shows grouped capability summaries for browse, mutation, sharing/versioning, and metadata behavior.
- Validation results include sanitized fix hints for common failures such as invalid local roots, missing targets, permission failures, timeouts, and invalid config.
- Validation results include MCP readiness notes for disabled storage, non-exposed storage, writable MCP exposure, and presigned download-link capability.
- Validation summaries are copyable without including raw credentials or full storage config.
- TypeScript and Rust validation models include versioning capability fields, fix hints, and warnings.
- New storage additions and imports default to not exposed to MCP, preserving explicit agent-access opt-in.
- Global search indexing can be stopped from the dialog; stale in-flight recursive list responses are ignored so slow storage responses do not overwrite newer UI state after cancellation, close, or unmount.

Remaining public v0.7.0 candidates:

- Additional WebDAV presets and compatibility toggles.
- Additional S3-compatible presets where they materially reduce setup friction.

Deferred:

- SFTP, until OpenDAL support, simulator coverage, private-key masking, and known-hosts UX meet the product-ready rule.
- FTP, until clear security warnings, FTPS expectations, and capability coverage are in place.
- TOS once OpenDAL exposes useful read/write/list/stat capability.

## Future quality work

- Continue frontend branch/function coverage hardening for `FileBrowser.tsx`, `FilePreviewPanel.tsx`, and `McpSettingsDialog.tsx`.
- Keep release artifact builds blocked behind the automated zero-manual release gate.
- Keep storage-simulator coverage focused on OpenDAL behavior and safe fallbacks instead of provider-specific SDK paths.
