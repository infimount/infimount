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

## HTTP Transport

For desktop and local development, keep HTTP bound to loopback:

```text
127.0.0.1
```

Headless HTTP mode requires bearer-token authentication unless explicitly started with `--allow-insecure`.
Set a token with either CLI or environment:

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

Filesystem tools that receive a `session_id` enforce those restrictions before backend operations.

## Backend Capability Boundaries

Some capabilities are backend-dependent:

- Object versions require backend and bucket/container support.
- Presigned download links require backend support.
- WebDAV does not expose object-version tools.

See [Backend Capability Matrix](backend-capabilities.md) for the public support matrix.
