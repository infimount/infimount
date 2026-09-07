# Infimount Agent Tasks for Codex

This is a skill-first Codex plugin for the Infimount v0.8.1 Agent Tasks workflow.

It intentionally does **not** bundle a second storage implementation or credentials. Configure the Infimount MCP sidecar for Codex, then install this plugin so Codex understands the task-package workflow and output boundary.

## Why skill-first

Codex plugins support a `.codex-plugin/plugin.json` manifest, `skills/`, and optional MCP configuration. Infimount keeps MCP installation in the desktop/client setup flow because the local sidecar path is installation-specific and must be verified by Infimount before use. This also avoids coupling the plugin to evolving bundled-stdio MCP configuration details.

## Expected user flow

1. In Infimount, select files and choose **Use with agent**.
2. Choose or create a local read/write Agent Workspace.
3. Review the preflight and prepare the task.
4. Copy the generated agent instruction, or provide Codex the task's Infimount MCP path.
5. Codex reads `TASK.md` and the prepared `inputs/` files, then writes deliverables under `outputs/`.
6. Return to Infimount Agent Tasks to preview/select outputs.
7. Publish only the approved, unchanged outputs through the desktop app.

## Plugin structure

```text
.codex-plugin/plugin.json
skills/agent-tasks/SKILL.md
README.md
```

The plugin does not need `.mcp.json` in this first version. Native Codex MCP setup can be added by the Infimount desktop adapter once its executable/config identity has been verified, without changing the task skill.

## Development install

Use the Codex plugin installation mechanism for a local/repository plugin and point it at this directory. The exact marketplace location depends on whether you are testing it as a personal plugin or as a repository/team plugin.

The Infimount MCP server must already be available to Codex. The task skill should never request raw S3, Drive, Azure, GCS, WebDAV, SFTP, or other storage credentials.
