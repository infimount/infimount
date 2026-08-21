# Backend Capability Matrix

Infimount uses OpenDAL capabilities at runtime. A backend being listed here does not mean every account, bucket, container, or server has every feature enabled.
Use the desktop **Validate** action to check the effective capabilities for a configured storage. Validation is a desktop control-plane operation and is not exposed through public MCP. It reports grouped browse, mutation, sharing/versioning, and metadata capabilities, plus sanitized fix hints and MCP readiness notes.

This matrix tracks the latest stable release and the current `main` branch.

## OpenDAL-First Storage Policy

OpenDAL is Infimount's storage abstraction boundary. File operations should stay routed through OpenDAL-backed core/MCP layers rather than provider-specific SDKs or custom per-provider implementations.

This means future features should be designed around capabilities exposed through OpenDAL. If a feature requires backend-specific primitives that are not available through the common OpenDAL path, prefer a backend-agnostic UX fallback, capability detection, or clear "not supported" state over adding provider-specific storage code.

For backend expansion, a backend is not considered product-ready until it has an OpenDAL operator builder, desktop schema, MCP builder support, capability coverage, automated tests, and documentation. Optional write-time user metadata is capability-gated: Infimount only sends requested metadata when OpenDAL reports `write_with_user_metadata`, and otherwise returns an explicit unsupported-backend error instead of silently dropping metadata. OpenDAL 0.56.0 also introduced a Volcengine TOS service crate, but its Rust backend currently reports no read/write/list/stat capability, so Infimount does not expose it as a user-selectable storage yet.

| Backend                   | Browse/read/write | Presigned download links | Object versions          | Notes                                                                       |
| ------------------------- | ----------------- | ------------------------ | ------------------------ | --------------------------------------------------------------------------- |
| Local filesystem          | Yes               | No                       | No                       | Local paths are direct filesystem operations.                               |
| Amazon S3 / S3-compatible | Yes               | Backend-dependent        | Backend/config-dependent | Versioning requires bucket support and versioning enabled. Optional `defaultAcl` is passed through OpenDAL for writes. |
| Backblaze B2              | Yes               | Yes                      | No                       | Native OpenDAL B2 backend. Supports capability-gated write-time user metadata when OpenDAL reports `write_with_user_metadata`. |
| Aliyun OSS                | Yes               | Yes                      | Backend/config-dependent | Object storage through OpenDAL. Generic rename and create-directory capabilities are not exposed. |
| Tencent COS               | Yes               | Yes                      | Backend/config-dependent | Object storage through OpenDAL. Generic rename and create-directory capabilities are not exposed. |
| Huawei OBS                | Yes               | Yes                      | Backend/config-dependent | Object storage through OpenDAL. Generic rename and create-directory capabilities are not exposed. |
| Azure Blob Storage        | Yes               | Backend-dependent        | Backend/config-dependent | Version behavior depends on account/container support and configuration.    |
| Google Cloud Storage      | Yes               | Backend-dependent        | Backend/config-dependent | Versioning requires object versioning/generation support and configuration. |
| Google Drive              | Yes               | No                       | No                       | OAuth-backed Google Drive through OpenDAL. The desktop app can guide connection with a local loopback OAuth + PKCE flow, or advanced users can provide tokens manually. See [Guided OAuth for Google Drive and Microsoft OneDrive](oauth-drive-setup.md). Tokens stay local. |
| Microsoft OneDrive        | Yes               | No                       | Config-dependent         | OAuth-backed OneDrive Personal through OpenDAL. The desktop app can guide connection with a local loopback OAuth + PKCE flow, or advanced users can provide tokens manually. See [Guided OAuth for Google Drive and Microsoft OneDrive](oauth-drive-setup.md). Enable `versioning` for version listing when the account supports it. Tokens stay local. |
| WebDAV                    | Yes               | No                       | No                       | Version tools return `ERR_VERSIONS_NOT_SUPPORTED`. Use `disableCreateDir` for servers that reject collection creation probes/placeholders. |
| SFTP                      | Yes               | No                       | No                       | Linux/macOS only. Key-based SFTP through OpenDAL. Password login is not exposed because the OpenDAL SFTP backend does not support it. Optional remote copy depends on server extension support and `enableCopy`. |
| FTP                       | Yes               | No                       | No                       | FTP through OpenDAL with username/password auth. Generic copy and rename are not exposed by the backend, so Infimount falls back to stream-copy plus delete for moves where safe. |

## Atomic No-Overwrite Writes

MCP `write_file` with `overwrite=false` requires an atomic create-if-absent write (`write_with_if_not_exists`). Backends that cannot guarantee atomic no-overwrite return `ERR_BACKEND_UNSUPPORTED` for no-overwrite writes instead of falling back to a stat-then-write sequence that could race and silently overwrite an existing object. Use the desktop **Validate** action to check a storage's effective capabilities, including atomic create-if-absent support.

## Error Semantics

Version-aware tools return deterministic MCP errors:

- `ERR_VERSIONS_NOT_SUPPORTED`: the backend cannot support versions.
- `ERR_VERSIONS_NOT_ENABLED`: the backend can support versions, but this storage is configured or detected as not version-enabled.
- `ERR_STORAGE_READ_ONLY`: a mutation was attempted on a read-only storage.
- `ERR_BACKEND_UNSUPPORTED`: a non-version capability such as presigned links is not available for that backend.

## Validation Output

The desktop **Validate** action reports:

- `valid`: whether the storage could be reached with the configured root, bucket, container, or prefix.
- `details`: a sanitized result message such as success, timeout, permission denied, missing target, or invalid local root.
- `capabilities`: OpenDAL-reported booleans for list, stat, read, write, delete, copy, rename, create directory, presigned reads, object versions, and write-time user metadata.
- `fix_hints`: actionable next steps that avoid echoing raw credentials or full secret-bearing config.
- `warnings`: advisory MCP readiness notes, such as disabled storage, not exposed to MCP, writable MCP exposure, or presigned-link capability.

Validation does not automatically change storage exposure or MCP settings.

## Recommended Validation Before Exposing a Storage

1. Add or edit the storage in Infimount.
2. Run Validate.
3. Confirm the effective capabilities match the intended MCP exposure.
4. Review MCP readiness notes, especially writable exposed storage or presigned download-link support.
5. Set `read_only=true` for storages that agents should not mutate.
6. Keep `mcp_exposed=false` for storages that should remain desktop-only.
