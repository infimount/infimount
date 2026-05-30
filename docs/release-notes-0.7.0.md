# Infimount 0.7.0: Storage Expansion and Validation Clarity

Infimount 0.7.0 expands the OpenDAL-backed storage set and makes storage validation clearer before a backend is used in the desktop app or exposed to MCP clients.

Release: <https://github.com/infimount/infimount/releases/tag/v0.7.0>

## Highlights

- Added first-class Aliyun OSS, Tencent COS, and Huawei OBS storage support across desktop Add/Edit Storage, Rust core builders, MCP builders, schemas, capability docs, and tests.
- Added capability-aware validation summaries in Add/Edit Storage with grouped browse, mutation, sharing/versioning, and metadata signals.
- Added sanitized fix hints and MCP readiness notes so users can understand reachability, read/write capability, exposure state, read-only risk, and presigned-link support without leaking storage config or secrets.
- Added copyable validation summaries that avoid raw credentials, auth tokens, storage config JSON, and file contents.
- Added a Stop control for global search indexing; cancelled or stale recursive list responses are ignored after cancellation, dialog close, or unmount.

## MCP and security behavior

- New storage additions and imported storages now default to `mcp_exposed=false`, preserving explicit agent-access opt-in.
- Legacy source migration also defaults migrated storages to not MCP-exposed.
- MCP storage-management tools accept backend aliases such as `aliyun_oss`, `tencent_cos`, `huawei_obs`, `backblaze_b2`, and `azblob`, then persist canonical backend names.
- Secret masking now covers additional snake_case and camelCase aliases, including service-account JSON fields.
- Validation warnings are advisory only. Storage exposure, enabled tools, read-only mode, path policy, and confirmations remain explicit user controls.

## Install and docs polish

- The GitHub Pages download flow now prioritizes single-command install cards for Linux, macOS, and Windows PowerShell.
- Install command snippets include copy buttons and avoid horizontal scrollbars.
- README installation guidance is organized by platform with mobile-friendlier manual download links.

## Notes

All storage operations remain OpenDAL-first. Capabilities are backend, account, bucket, container, and server dependent. Use Validate before browsing a new storage or exposing it to MCP clients.
