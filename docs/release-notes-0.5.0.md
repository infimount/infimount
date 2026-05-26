# Infimount 0.5.0: Backend Expansion

Infimount 0.5.0 expands the OpenDAL-backed storage surface while preserving the local-first, safe-MCP architecture.

## Highlights

- Native Backblaze B2 support across desktop, core, and MCP.
- S3 `defaultAcl` configuration for buckets that require default object ACLs.
- WebDAV `disableCreateDir` compatibility mode for servers that reject collection creation probes/placeholders.
- Capability reporting for `write_with_user_metadata`.
- Capability-gated user metadata writes through desktop APIs and MCP `write_file`.
- `stat_path` returns user metadata when OpenDAL exposes it.
- OpenDAL 0.56.0 upgrade with Rust 1.85+ documented as the source-build minimum.

## Backend expansion policy

Every new backend or backend-specific option continues to follow:

```text
new backend = OpenDAL operator + capability matrix + tests + docs
```

No direct provider SDKs were added for file operations.

## Notes

Volcengine TOS was assessed through OpenDAL 0.56.0 but is not exposed yet because the current Rust service does not report product-ready read/write/list/stat capability.
