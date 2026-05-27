# Security Model

Infimount is local-first by design. It does not require an Infimount-hosted backend to store your storage registry, runtime settings, or credentials.

## Local Data Storage

Default local files:

- `~/.infimount/storages.json`: storage registry and backend configuration.
- `~/.infimount/mcp_settings.json`: MCP runtime settings, transport, bind address, port, auth token, and enabled tool list.

Treat these files as sensitive because storage credentials can be present in backend configuration.

## Secret Handling

Infimount masks secrets in storage-management outputs by default.

- `list_storages` returns masked secret values.
- `export_config` masks secrets unless explicitly called with `include_secrets=true`.
- UI and MCP logs should not print raw storage config JSON or raw input payloads.
- Browser/admin-style views should replace secrets instead of revealing them by default.

## MCP Exposure Controls

A storage is visible to MCP only when both flags are true:

- `enabled=true`
- `mcp_exposed=true`

Set `read_only=true` to prevent write, delete, move, and version-delete operations for that storage.

MCP settings also include an enabled-tool list. Disable tools such as `export_config`, `import_config`, `add_storage`, or `delete_path` when a client only needs read access.

Tool exposure changes apply after restarting the MCP HTTP server.

## Path Policies and Confirmations

Each storage can define a local MCP policy:

- access mode: no access, read-only, or read/write
- allowed path prefixes
- denied path prefixes
- confirmation rules for risky operations

Denied prefixes always win over allowed prefixes. Prefix matching is segment-aware and paths are normalized before policy checks so repeated slashes, trailing slashes, `.` / `..`, and URL-encoded-looking control segments such as `%2e` and `%2f` cannot bypass a deny rule. Matching remains case-sensitive because backend case behavior is not globally consistent.

Risky operations can return `requires_confirmation` instead of executing. The pending operation includes an immutable request fingerprint. Approval is valid once, expires after a bounded TTL, and cannot be replayed for a modified request. Pending approvals are in-memory runtime state and are cleared by an app/server restart.

By default, confirmations are required for:

- writes and overwrites
- deletes
- version deletes
- presigned/download-link generation
- cross-storage copy
- rename/move operations that may behave like copy plus delete

Desktop notifications, when enabled by the user, are attention signals only. They do not approve or deny operations and do not include tokens, secrets, or presigned URLs.

## MCP Audit Log

Infimount stores a bounded local MCP audit log at `~/.infimount/mcp_audit.json`.

Audit events include tool name, storage metadata when available, operation, path, decision, confirmation ID, duration, and error code. The audit log records allowed, denied, confirmation-required, confirmed, and failed operations.

Safety rules:

- auth tokens are not logged
- storage secrets are not logged
- file contents are not logged
- presigned URLs have query strings redacted before persistence and export
- sensitive headers are not logged

The desktop audit viewer can export the current filtered rows as a local JSON bundle under `~/.infimount/exports/`. The bundle includes a redaction manifest stating that secrets, file contents, auth tokens, and presigned URL query strings are excluded or redacted.

## HTTP Transport

For desktop and local development, keep HTTP bound to loopback:

```text
127.0.0.1
```

Desktop HTTP can run unauthenticated only on loopback for local development. If you bind desktop HTTP to `0.0.0.0` or a LAN address, Infimount requires a bearer token before the server can start.

Headless HTTP mode also requires bearer-token authentication unless explicitly started with `--allow-insecure` on loopback. Set a token with either CLI or environment:

```bash
INFIMOUNT_AUTH_TOKEN='replace-with-a-random-token' infimount_mcp --transport http --bind 127.0.0.1 --port 7331
```

Clients must send:

```text
Authorization: Bearer replace-with-a-random-token
```

Only bind to `0.0.0.0` or a LAN address when you intentionally expose the server and have a strong token plus a network boundary in place.

## Sessions and Scoped Access

MCP clients can create scoped sessions with:

- allowed storage names
- optional allowed path prefixes
- optional read-only override
- TTL

Filesystem tools that receive a `session_id` enforce those restrictions before backend operations. Session path prefixes are normalized and segment-aware to avoid broad matches such as allowing `docs2` when only `docs` was scoped. Desktop MCP Settings shows active in-memory scoped sessions with their storage scope, path prefixes, read-only status, and expiry so users can inspect current agent scopes. Desktop HTTP sessions are cleared when that server stops.

## Storage Validation Safety

The desktop Validate action and MCP `validate_storage` tool report reachability, effective capabilities, sanitized fix hints, and MCP readiness notes. Validation output must not include raw credentials, auth tokens, storage config JSON, or file contents. MCP readiness notes are advisory only; users must still explicitly enable storage exposure, tool access, path policy, read-only mode, and confirmations.

## Backend Capability Boundaries

Some capabilities are backend-dependent:

- Object versions require backend and bucket/container support.
- Presigned download links require backend support.
- WebDAV does not expose object-version tools.

See [Backend Capability Matrix](backend-capabilities.md) for the public support matrix.
