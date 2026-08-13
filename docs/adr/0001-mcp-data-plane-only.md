# ADR 0001: MCP Data-Plane-Only Public Tool Surface

**Status:** Accepted
**Date:** 2026-07-18
**Driver:** v0.8 Trust & Activation

## Context

The current public MCP server exposes both data-plane tools (file read/write)
and storage-administration tools (add/remove/edit storages, import/export
config). This means any MCP client — an AI agent with the user's approval —
can manage storage registries, a power that should be reserved for the
desktop control plane.

Users expect that granting MCP access exposes only files, not storage
configuration or credentials.

## Decision

The public MCP server will expose filesystem/data-plane tools only.
Storage-administration tools are removed from:

- `tool_definitions()`
- `dispatch_tool_json()`
- MCP schemas reachable by public discovery
- Helper invoke functions intended only for MCP dispatch

The desktop control plane continues to call the underlying Rust functions
directly through Tauri commands.

## Affected files

- `crates/mcp/src/server.rs`
- `crates/mcp/src/settings.rs`
- `crates/mcp/src/schemas.rs`
- `crates/mcp/src/lib.rs`
- `crates/mcp/src/tools_storage/*`
- `apps/desktop/src/components/mcp/McpToolSection.tsx`
- `apps/desktop/src/components/mcp/McpSettingsDialog.tsx`
- `apps/desktop/src/types/storage.ts`
- `apps/desktop/src/pages/Index.tsx`
- `docs/mcp-client-setup.md`
- `docs/security.md`

## Contract

```rust
// Tool metadata additions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolCategory {
    Read,
    Write,
    Destructive,
    ExternalLink,
    Session,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
    pub category: McpToolCategory,
    pub risk: McpToolRisk,
    pub default_enabled: bool,
}
```

No `Administration` category is added to the public data-plane server.

### Tool exposure

Removed from public MCP (administrative tools):

```
list_storages
add_storage
edit_storage
remove_storage
import_config
export_config
validate_storage
```

Default enabled (safe read-only set):

```
list_dir
stat_path
read_file
search_paths
list_versions
read_file_version
```

Disabled by default:

```
mkdir
write_file
copy_path
move_path
delete_path
delete_version
generate_download_link
session_create
session_end
```

### Settings migration

- Before mutation, copy `mcp_settings.json` to
  `backups/mcp_settings.pre-v0.8.<timestamp>.json`.
- If `securityBaselineVersion` is missing or less than 2, replace
  `enabledTools` with the intersection of the old list and the safe
  default set.
- If the intersection is empty, use the complete safe default set.
- Set version 2 and persist.

### PR 01 interim HTTP auth-token wire plumbing (removed in PR 03)

During PR 01 the frontend HTTP-token plumbing is corrected temporarily:

- `McpSettingsWire` includes `authToken`.
- `mapStatusWire()` maps it.
- `handleSaveMcpSettings()` sends it.

PR 03 removes raw token transport entirely. No intermediate release ships
with the interim wire.

### MCP tool annotation rules

Each data-plane tool MUST carry the following annotations:

- **read tools** (`list_dir`, `stat_path`, `read_file`, `search_paths`,
  `list_versions`, `read_file_version`): read-only, non-destructive,
  idempotent.
- `generate_download_link`: read-only, non-destructive, idempotent,
  open-world.
- **write tools** (`mkdir`, `write_file`, `copy_path`): not read-only.
- **destructive tools** (`move_path`, `delete_path`, `delete_version`):
  destructive.
- **session tools** (`session_create`, `session_end`): non-destructive
  but NOT default-enabled.

Annotations are used by the frontend for category grouping, risk labels,
and confirmation-dialog triggers (see Frontend changes below).

### Frontend changes

- Remove the "Enable all" action.
- Replace with "Apply safe read-only" and "Configure advanced tools".
- Group tools by category; show medium/high-risk labels.
- Enabling a write, destructive, external-link, or session tool requires
  a confirmation dialog.

## Consequences

- MCP clients cannot discover or invoke registry administration.
- A fresh installation exposes no write/destructive tool by default.
- Existing settings are safely migrated with a timestamped backup.
- Documentation marks the MCP admin-tool removal as a pre-1.0 breaking
  change.
- Desktop commands retain full administrative capability.
