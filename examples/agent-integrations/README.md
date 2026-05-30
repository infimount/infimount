# Agent Integration Examples

These examples show how to connect agents to Infimount through MCP.

- `claude-desktop-stdio.json`: stdio MCP server config.
- `generic-http.json`: local HTTP MCP server config.
- `pi-infimount-extension/`: Pi extension starter that wraps Infimount MCP tools.

Keep Infimount as the policy boundary. Do not give an agent direct cloud credentials when it can use an Infimount-exposed, read-only, scoped storage instead.
