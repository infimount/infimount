# Agent Integrations

Infimount is best used by coding agents through its MCP server. Keep Infimount as the storage boundary: configure storages in the desktop app, expose only the storages agents need, keep read-only mode on when possible, and let Infimount enforce path policies, confirmations, sessions, and audit logs.

## Recommended setup

1. Add and validate storage in Infimount.
2. Leave new storages unexposed to MCP until you explicitly need agent access.
3. In **MCP Settings**, expose only the target storage or workspace root.
4. Prefer read-only access for research/review agents.
5. Use confirmations for writes, deletes, moves, and download links.
6. Review the audit log after agent work.

## Generic stdio MCP config

Use this for clients that launch MCP servers themselves, including Claude Desktop and other MCP-compatible agent tools.

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

If `infimount_mcp` is not on `PATH`, use the absolute path to the binary.

## Generic local HTTP MCP config

Use this when Infimount is already running the MCP HTTP server locally.

```json
{
  "mcpServers": {
    "infimount": {
      "url": "http://127.0.0.1:7331/mcp"
    }
  }
}
```

If you bind HTTP outside loopback, Infimount requires a bearer token. Configure the client header:

```json
{
  "Authorization": "Bearer replace-with-a-random-token"
}
```

## Claude Desktop

Add the stdio config to Claude Desktop's MCP server configuration:

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

Restart Claude Desktop after editing the config.

## Claude Code

If your Claude Code install supports MCP commands, add Infimount as a local stdio server:

```bash
claude mcp add infimount -- infimount_mcp --transport stdio
```

For safer project work, create an Infimount Agent Workspace first, apply the workspace-scoped MCP policy, then connect Claude Code.

## OpenCode

OpenCode supports MCP directly. Add an `opencode.jsonc` file to a project or your OpenCode config with a local Infimount MCP server:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "infimount": {
      "type": "local",
      "command": ["infimount_mcp", "--transport", "stdio"],
      "enabled": true,
      "timeout": 10000
    }
  }
}
```

For an already-running local HTTP server:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "infimount": {
      "type": "remote",
      "url": "http://127.0.0.1:7331/mcp",
      "headers": {
        "Authorization": "Bearer replace-with-a-random-token"
      },
      "enabled": true,
      "timeout": 10000
    }
  }
}
```

Example files are included at:

```text
examples/agent-integrations/opencode-local.jsonc
examples/agent-integrations/opencode-http.jsonc
```

Validation commands:

```bash
opencode debug config --pure
opencode mcp list --pure
```

The local stdio setup has been smoke-tested with OpenCode: OpenCode connected to the Infimount MCP server and invoked `infimount_read_file` against a temporary local storage.

## Codex-style agents

For Codex-style CLIs and editor agents, use the client's MCP configuration mechanism if available and point it at Infimount using either:

- stdio: `infimount_mcp --transport stdio`
- local HTTP: `http://127.0.0.1:7331/mcp`

If the client does not support MCP directly, use a small adapter or extension that calls Infimount MCP tools and does not bypass Infimount policy.

## Pi extension starter

Pi does not ship built-in MCP support, so the recommended path is a Pi extension that talks to Infimount MCP. A starter extension is included at:

```text
examples/agent-integrations/pi-infimount-extension/
```

It is structured as a publishable Pi package (`@infimount/pi-infimount-extension`) and registers read-first Infimount tools for Pi:

- `infimount_list_storages`
- `infimount_list_dir`
- `infimount_read_file`
- `infimount_search_paths`
- `infimount_generate_download_link`

Install it into Pi's extension directory or load it for testing:

```bash
cd examples/agent-integrations/pi-infimount-extension
npm install
npm run smoke
pi -e ./index.ts
```

Or install the local package into Pi:

```bash
pi install ./examples/agent-integrations/pi-infimount-extension
```

The smoke test creates a temporary local storage and verifies list/read/search calls through Infimount MCP. `npm pack --dry-run` verifies the package contents for publishing.

The extension uses `infimount_mcp --transport stdio` by default. Override the command with:

```bash
INFIMOUNT_MCP_COMMAND=/absolute/path/to/infimount_mcp pi -e ./index.ts
```

Use a separate Infimount config home for tests or sandboxes:

```bash
INFIMOUNT_MCP_HOME=/tmp/infimount-fixture pi -e ./index.ts
```

Set `INFIMOUNT_MCP_STDERR=inherit` if you need to debug the child MCP server. Stderr is hidden by default to keep Pi answers clean.

## Safe defaults for agents

Use these defaults unless a task needs more access:

- expose one storage or one workspace root only
- set storage or policy to read-only
- disable storage-management tools for normal coding agents
- require confirmations for write, delete, move, version-delete, and download-link operations
- prefer short-lived sessions for scoped tasks
