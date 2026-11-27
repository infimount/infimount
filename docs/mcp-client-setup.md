# MCP Client Setup

Infimount can expose configured storages to MCP clients through either stdio or local Streamable HTTP.
All paths use the Infimount virtual filesystem:

- `/` lists MCP-exposed storages.
- `/StorageName` lists a storage root.
- `/StorageName/path/to/file.txt` routes to that backend path.

Only storages with `enabled=true` and `mcp_exposed=true` are visible to MCP tools.
If a storage is marked `read_only=true`, write tools are rejected for that storage.

## Data-Plane Only: No Storage Administration via MCP

The public MCP server exposes **filesystem/data-plane tools only**. Storage administration
tools (list_storages, add_storage, edit_storage, remove_storage, import_config, export_config,
validate_storage) are **not available** through MCP discovery or dispatch.

These functions remain available exclusively through the desktop control plane (the Infimount
desktop application). This means MCP clients — including AI agents — cannot manage storage
registries or access credentials through MCP.

This is a pre-1.0 breaking change from v0.7. See [Security Model](security.md) for details.

## Safe Default Tool Set

A fresh installation enables only the safe read-only tool set by default:

- list_dir
- stat_path
- read_file
- search_paths
- list_versions
- read_file_version

All write, destructive, external-link, and session tools are disabled by default and must be
explicitly enabled. Each tool is annotated with a category (Read, Write, Destructive,
ExternalLink, Session) and a risk level (Low, Medium, High).

## Desktop Settings

Open **MCP Settings** in the desktop app to configure:

- transport: `stdio` or `http`
- bind address and port for HTTP
- exposed tool list (grouped by category with risk labels)
- generated client snippets
- per-storage path policies and confirmation rules
- pending approval queue and MCP audit viewer

Apply an access preset to configure tools and policies together: **Read-only research** for safe reads, **Workspace Agent** for non-destructive writes inside existing workspace grants, **Manual Approval** for broad tools with explicit confirmations, or **Lock down MCP** to pause agent access. Use **Configure advanced tools** to inspect non-default tools. Enabling any write, destructive, external-link, or session tool requires a confirmation dialog, regardless of its risk label.

Tool exposure changes are applied after restarting the HTTP server. The settings panel shows when a restart is required.

## What the Agent Can Access

MCP access is the intersection of several local controls:

1. The storage must be enabled.
2. The storage must be exposed to MCP.
3. The requested tool must be enabled.
4. The storage policy must allow the requested path.
5. Read-only storage or read-only policy must allow the operation type.
6. Path rules grant explicit access for specific prefixes; if no rule matches, the default access mode applies.
7. Risky operations may require approval before execution.

Use the **What the agent can access** summary in MCP Settings before connecting a client. Denied path prefixes override all other rules. When no path rules are defined, the default access mode applies to all paths.

## Risky Operation Approval

When a tool call requires approval, the MCP response contains `status: "requires_confirmation"` and an operation ID. The operation is not executed until it is approved in Infimount.

The approval queue shows:

- tool name
- operation and risk type
- storage
- path
- exact action summary
- expiry time

MCP Settings also shows active scoped sessions created by MCP clients, including storage scope, path prefixes, read-only status, and expiry. These sessions are in-memory, expire locally, and are cleared when the desktop HTTP server stops. Audit events include the matched policy rule ID and workspace ID when a policy rule determined the access decision.

Approvals are single-use and tied to the original request fingerprint. A client cannot approve one operation and then reuse the ID for a different path, storage, or tool. Pending approvals are in-memory and are cleared by app/server restart.

## Audit Viewer

MCP Settings includes a local audit viewer for recent MCP activity. It records allowed, denied, confirmation-required, confirmed, and failed tool calls. The audit log is bounded and local-only. Secrets, auth tokens, file contents, and presigned URL query signatures are not stored.

Use **Copy visible** for a JSON clipboard export of the current filtered rows, or **Export visible** to write a local redacted audit bundle under `~/.infimount/exports/` with a redaction manifest.

## Claude Desktop / Stdio

Use stdio when the MCP client launches the server process itself. Copy the generated JSON from MCP Settings; the desktop inserts the verified absolute path of its bundled, same-version sidecar and correctly escapes spaces and Windows backslashes.

```json
{
  "mcpServers": {
    "infimount": {
      "command": "/absolute/path/from-Infimount-MCP-Settings/infimount_mcp",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

Normal desktop users do not need to build or install `infimount_mcp` separately, and onboarding does not fall back to an executable on `PATH`.

## Cursor / VS Code-style MCP JSON

Many editor clients accept a similar MCP JSON shape. Use stdio for the broadest compatibility:

```json
{
  "mcpServers": {
    "infimount": {
      "command": "/absolute/path/from-Infimount-MCP-Settings/infimount_mcp",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

When the client supports Streamable HTTP, use the HTTP URL form instead:

```json
{
  "mcpServers": {
    "infimount": {
      "url": "http://127.0.0.1:7331/mcp"
    }
  }
}
```

## LM Studio / Generic HTTP Clients

Use HTTP when Infimount is already running a local server.
The default endpoint is:

```text
http://127.0.0.1:7331/mcp
```

Port `0` is supported for auto-pick. In that case, use the actual endpoint shown by the desktop settings panel or the `infimount_mcp` process output.

Generic HTTP configuration:

```json
{
  "name": "infimount",
  "transport": "http",
  "url": "http://127.0.0.1:7331/mcp"
}
```

## HTTP Authentication

Desktop HTTP can run without a token only on loopback for local development. The built-in desktop and sidecar HTTP transports are loopback-only. Use a TLS-terminating reverse proxy for any remote deployment.

Headless HTTP mode requires a bearer token unless `--allow-insecure` is passed for loopback local development. Set the token with either CLI or environment:

```bash
INFIMOUNT_AUTH_TOKEN='replace-with-a-random-token' /verified/path/to/infimount_mcp serve --transport http --bind 127.0.0.1 --port 7331
```

Clients must send:

```text
Authorization: Bearer replace-with-a-random-token
```

If your client has a headers field, configure it like this:

```json
{
  "Authorization": "Bearer replace-with-a-random-token"
}
```

Keep the default bind address at `127.0.0.1` for desktop/local use. Binding to `0.0.0.0` or a LAN address is rejected by the built-in server.

## Available Tools

The exact exposed tool list is controlled from MCP settings.
Current tool groups include:

- filesystem: `list_dir`, `stat_path`, `read_file`, `write_file`, `mkdir`, `copy_path`, `move_path`, `delete_path`
- versions: `list_versions`, `read_file_version`, `delete_version`
- utility: `search_paths`, `generate_download_link`
- sessions: `session_create`, `session_end`

Version listing and write operations are bounded for safety:

- `list_versions` scans at most 10,000 versions per path, returns at most 1,000 per page, and reports `truncated: true` when more versions may exist. Pagination uses a signed cursor bound to the storage, path, and storage revision.
- `write_file` is disabled by default. Send UTF-8 text with the default `encoding: "utf-8"`, or binary files (images, spreadsheets, archives) as standard padded base64 with `encoding: "base64"`. Limits apply to decoded bytes. `read_file` with `as_text: false` returns the same base64 form. No-overwrite writes require an atomic create-if-absent-capable backend. Unsupported write guarantees or metadata return `ERR_BACKEND_UNSUPPORTED` before parent directories or files are created.

### File byte limits and content type

Advanced MCP settings exposes positive, whole-byte limits. The settings persist
in `mcp_settings.json` and apply to ordinary stdio/HTTP MCP, resource reads and
Agent Task MCP. Restart an existing server or Agent Task sidecar to pick up a
change. Storage-only browsing and publication approval keep their existing
contracts.

| Settings field | Default and hard maximum | Tool behavior |
| --- | --- | --- |
| `maxReadBytes` | 2,097,152 bytes (2 MiB) | Bounds `read_file` and `read_file_version`. An omitted `max_bytes` uses the smaller of 262,144 bytes and this setting. Explicit values above the configured cap fail. |
| `maxWriteBytes` | 4,194,304 bytes (4 MiB) | Bounds UTF-8 byte length and decoded base64 bytes for `write_file`. |

These settings may lower the existing ceilings. Zero, fractional values and
values above the ceilings are rejected. Old settings without the fields use
the defaults. Tool discovery reports the effective limits. Size, encoding and
content-type checks run before an approval is queued. An approval binds the
exact request, including metadata, and the active limits, so changing either
invalidates it.

`write_file` accepts optional `content_type`, for example `image/png` or
`text/plain; charset=utf-8`. It must be a valid ASCII media type of at most 255
bytes. The backend must advertise OpenDAL `write_with_content_type`, and the
value is passed to that writer. A local filesystem cannot persist this object
metadata and rejects an explicit content type. Omitting the field preserves
the existing write behavior.

`read_file` and `read_file_version` include optional `content_type` when the
backend returns a valid value in its metadata. The field is absent when the
backend provides no usable content type. The value is not guessed from the
filename or content. Binary content remains base64 regardless of media type.

Storage-management operations are desktop-only and are never exposed as public MCP tools.
For threat model details, see [Security Model](security.md).
