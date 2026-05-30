# Agent Integration Examples

These examples show how to connect agents to Infimount through MCP.

- `claude-desktop-stdio.json`: stdio MCP server config.
- `generic-http.json`: generic local HTTP MCP server config.
- `opencode-local.jsonc`: OpenCode local stdio MCP config.
- `opencode-http.jsonc`: OpenCode local HTTP MCP config.
- `pi-infimount-extension/`: Pi extension starter that wraps Infimount MCP tools.

Keep Infimount as the policy boundary. Do not give an agent direct cloud credentials when it can use an Infimount-exposed, read-only, scoped storage instead.
