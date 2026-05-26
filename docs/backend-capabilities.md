# Backend Capability Matrix

Infimount uses OpenDAL capabilities at runtime. A backend being listed here does not mean every account, bucket, container, or server has every feature enabled.
Use `validate_storage` in MCP or the desktop Validate action to check the effective capabilities for a configured storage.

## OpenDAL-First Storage Policy

OpenDAL is Infimount's storage abstraction boundary. File operations should stay routed through OpenDAL-backed core/MCP layers rather than provider-specific SDKs or custom per-provider implementations.

This means future features should be designed around capabilities exposed through OpenDAL. If a feature requires backend-specific primitives that are not available through the common OpenDAL path, prefer a backend-agnostic UX fallback, capability detection, or clear "not supported" state over adding provider-specific storage code.

For backend expansion, a backend is not considered product-ready until it has an OpenDAL operator builder, desktop schema, MCP builder support, capability coverage, automated tests, and documentation. Optional write-time user metadata is capability-gated: Infimount only sends requested metadata when OpenDAL reports `write_with_user_metadata`, and otherwise returns an explicit unsupported-backend error instead of silently dropping metadata. OpenDAL 0.56.0 also introduced a Volcengine TOS service crate, but its Rust backend currently reports no read/write/list/stat capability, so Infimount does not expose it as a user-selectable storage yet.

| Backend                   | Browse/read/write | Presigned download links | Object versions          | Notes                                                                       |
| ------------------------- | ----------------- | ------------------------ | ------------------------ | --------------------------------------------------------------------------- |
| Local filesystem          | Yes               | No                       | No                       | Local paths are direct filesystem operations.                               |
| Amazon S3 / S3-compatible | Yes               | Backend-dependent        | Backend/config-dependent | Versioning requires bucket support and versioning enabled. Optional `defaultAcl` is passed through OpenDAL for writes. |
| Backblaze B2              | Yes               | Yes                      | No                       | Native OpenDAL B2 backend. Supports capability-gated write-time user metadata when OpenDAL reports `write_with_user_metadata`. |
| Azure Blob Storage        | Yes               | Backend-dependent        | Backend/config-dependent | Version behavior depends on account/container support and configuration.    |
| Google Cloud Storage      | Yes               | Backend-dependent        | Backend/config-dependent | Versioning requires object versioning/generation support and configuration. |
| WebDAV                    | Yes               | No                       | No                       | Version tools return `ERR_VERSIONS_NOT_SUPPORTED`. Use `disableCreateDir` for servers that reject collection creation probes/placeholders. |

## Error Semantics

Version-aware tools return deterministic MCP errors:

- `ERR_VERSIONS_NOT_SUPPORTED`: the backend cannot support versions.
- `ERR_VERSIONS_NOT_ENABLED`: the backend can support versions, but this storage is configured or detected as not version-enabled.
- `ERR_STORAGE_READ_ONLY`: a mutation was attempted on a read-only storage.
- `ERR_BACKEND_UNSUPPORTED`: a non-version capability such as presigned links is not available for that backend.

## Recommended Validation Before Exposing a Storage

1. Add or edit the storage in Infimount.
2. Run Validate.
3. Confirm the effective capabilities match the intended MCP exposure.
4. Set `read_only=true` for storages that agents should not mutate.
5. Keep `mcp_exposed=false` for storages that should remain desktop-only.
