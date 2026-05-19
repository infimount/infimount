# Safe MCP Storage Access: P0 Implementation Note

This note records the Phase 0 discovery for the Safe MCP Storage Access release work. It is intended to travel with the first P0 PR so reviewers can see where the implementation hooks into the existing architecture.

## Current Control Points

- **Tauri commands** live in `apps/desktop/src-tauri/src/commands.rs`. Desktop file operations still call `crates/core` directly, while MCP settings and storage import/export commands call `crates/mcp`.
- **Desktop runtime state** lives in `apps/desktop/src-tauri/src/state.rs`. It owns `StorageRegistry`, `McpSettingsStore`, and the optional in-process HTTP MCP runtime.
- **MCP server registration** lives in `crates/mcp/src/server.rs`. `tool_definitions()` is the single list of MCP tool names and schemas. Disabled tools are filtered from discovery and rejected before dispatch.
- **MCP filesystem tools** live under `crates/mcp/src/tools_fs/`. Each tool parses the absolute virtual path, resolves storage, checks session access, and then builds an OpenDAL operator.
- **MCP storage-management tools** live under `crates/mcp/src/tools_storage/`.
- **OpenDAL wiring** lives in `crates/mcp/src/opendal_adapter.rs` for MCP and `crates/core/src/operations.rs` for desktop file-manager operations.
- **Storage config** is `~/.infimount/storages.json`, represented by `StorageRecord` in `crates/mcp/src/registry.rs`. Records already include `enabled`, `mcp_exposed`, and `read_only`.
- **MCP runtime settings** are `~/.infimount/mcp_settings.json`, represented by `McpSettings` in `crates/mcp/src/settings.rs`. This already stores transport, bind/port, enabled tools, and optional auth token.
- **Runtime session state** is in-memory via `SessionManager` in `crates/mcp/src/session.rs`. HTTP uses a long-lived manager. Tauri command helper contexts currently create a fresh manager.
- **MCP Settings UI** is `apps/desktop/src/components/McpSettingsDialog.tsx`, opened from `apps/desktop/src/pages/Index.tsx`.
- **Frontend API hub** is `apps/desktop/src/lib/api.ts`; wire types are in `apps/desktop/src/types/storage.ts`.
- **Existing tests** include Rust tests in `crates/mcp`, React/Vitest component and integration tests under `apps/desktop/src`, and Playwright component tests through the existing desktop package scripts.

## P0 Implementation Shape

P0 should not add a separate MCP architecture. The intended insertion points are:

1. Add a storage-level MCP policy model to `StorageRecord` with serde defaults so existing `storages.json` files continue to load.
2. Add a central policy evaluator in `crates/mcp/src/policy.rs`.
3. Add a shared authorization helper in `crates/mcp/src/tools_fs/common.rs` and call it from filesystem/version/presign tools after path resolution and before OpenDAL execution.
4. Add confirmation state in `crates/mcp/src/confirmation.rs`, owned by the MCP server runtime and shared with the desktop `AppState` for in-process HTTP approvals.
5. Add bounded audit persistence in `crates/mcp/src/audit.rs`, written by the MCP server/runtime and exposed through Tauri commands.
6. Add Tauri commands for onboarding state, policy editing, pending confirmations, and audit reads.
7. Extend `McpSettingsDialog` instead of introducing a separate settings surface. The wizard can be a tab/section in the existing dialog.
8. Add first-run onboarding at the app shell level in `Index.tsx`, backed by a small local app settings file under `~/.infimount`.

## Testing Targets

- Unit test policy decisions before wiring tools.
- Unit test confirmation lifecycle: require, approve, deny, expire, replay.
- Unit test audit masking and bounded persistence.
- Integration test MCP tool calls for allowed/denied paths and disabled tools.
- UI tests for onboarding, MCP setup wizard, policy summary, confirmation queue, and audit viewer.

## Final P0 Status

Implemented for the 0.3 release candidate:

- First-run onboarding backed by local app settings.
- MCP setup/test entry points inside the existing MCP Settings dialog.
- Tool-level MCP exposure controls with disabled tools hidden from discovery and rejected before dispatch.
- Per-storage MCP policies with access mode, allowed prefixes, denied prefixes, and confirmation rules.
- Path normalization for policy checks and virtual path parsing, including repeated slashes, trailing slashes, `.` / `..`, backslashes, and URL-encoded-looking `%2e`, `%2f`, and `%5c` control segments.
- Risky-operation confirmation queue for write/delete/presign/version-delete/cross-storage copy/move-style operations.
- Single-use confirmation IDs tied to immutable request fingerprints.
- In-memory pending confirmation behavior. Restarting the app/server clears pending approvals.
- Bounded local audit persistence in `~/.infimount/mcp_audit.json`.
- Audit masking for presigned URL query strings.
- MCP Settings UI sections for runtime status, exposed storage summary, enabled tools, path policies, pending approvals, and audit events.

Known limitations intentionally left outside P0:

- Desktop notifications are optional attention signals only and do not approve or deny operations.
- Pending confirmations are not restored after restart.
- Path/name indexing, transfer queue, SFTP/backend expansion, and broader file-manager improvements are deferred.
- Audit storage is bounded local JSON, not a tamper-proof external log.
