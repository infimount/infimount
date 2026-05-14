# MCP Client Setup

Infimount can expose configured storages to MCP clients through either stdio or local Streamable HTTP.
All paths use the Infimount virtual filesystem:

- `/` lists MCP-exposed storages.
- `/StorageName` lists a storage root.
- `/StorageName/path/to/file.txt` routes to that backend path.

Only storages with `enabled=true` and `mcp_exposed=true` are visible to MCP tools.
If a storage is marked `read_only=true`, write tools are rejected for that storage.

## Desktop Settings

Open **MCP Settings** in the desktop app to configure:

- transport: `stdio` or `http`
- bind address and port for HTTP
- exposed tool list
- generated client snippets

Tool exposure changes are applied after restarting the HTTP server. The settings panel shows when a restart is required.

## Claude Desktop / Stdio

Use stdio when the MCP client launches the server process itself.

```json
{
  "mcpServers": {
    "infimount": {
      "command": "infimount_mcp",
      "args": ["--transport", "stdio"]
    }
  }
}
```

If `infimount_mcp` is not on `PATH`, use the absolute binary path in `command`.

## Cursor / VS Code-style MCP JSON

Many editor clients accept a similar MCP JSON shape. Use stdio for the broadest compatibility:

```json
{
  "mcpServers": {
    "infimount": {
      "command": "infimount_mcp",
      "args": ["--transport", "stdio"]
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

Headless HTTP mode requires a bearer token unless `--allow-insecure` is passed for local development.
Set the token with either CLI or environment:

```bash
INFIMOUNT_AUTH_TOKEN='replace-with-a-random-token' infimount_mcp --transport http --bind 127.0.0.1 --port 7331
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

Keep the default bind address at `127.0.0.1` for desktop/local use. Binding to `0.0.0.0` or a LAN address exposes the server outside the machine and should only be done with a strong token and an explicit network boundary.

## Available Tools

The exact exposed tool list is controlled from MCP settings.
Current tool groups include:

- filesystem: `list_dir`, `stat_path`, `read_file`, `write_file`, `mkdir`, `copy_path`, `move_path`, `delete_path`
- versions: `list_versions`, `read_file_version`, `delete_version`
- utility: `search_paths`, `generate_download_link`
- storage management: `list_storages`, `add_storage`, `edit_storage`, `remove_storage`, `import_config`, `export_config`, `validate_storage`
- sessions: `session_create`, `session_end`

Disable storage-management tools such as `export_config` if a client only needs filesystem access.
For threat model details, see [Security Model](security.md).
