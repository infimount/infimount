# Infimount 0.4.0: Workbench, Reliability, and Agent Workspaces

Infimount 0.4.0 is the next public release after 0.3.0. It bundles the completed workbench, reliability, and agent-workspace milestones into one coherent release.

## Transfer Workbench

- Transfer queue for copy and move operations.
- Sequential execution with retry, queued cancellation, and active transfer cancellation.
- Real Tauri transfer progress events with current path, item totals, and byte totals.
- Persisted transfer history with interrupted transfers restored as failed and retryable.
- Split-pane browsing with independent destination pane selection.
- Pane-to-pane copy and move actions.
- Conflict handling for fail, overwrite, skip, and keep-both auto-rename.
- Folder bookmarks, recent folders, and provider presets for common S3/WebDAV-compatible services.

## Workbench Reliability

- OpenDAL-only transfer dry-run manifests for copy and move planning.
- Dry-run summaries in the transfer queue before execution.
- Activity log events for planned, started, completed, failed, cancelled, recovery-started, and workspace operations.
- Interrupted-transfer recovery behavior that skips completed files when safe.
- Recursive OpenDAL metadata scans.
- Opt-in local path/name/metadata indexing per storage.
- Global search across indexed storages.
- Dual-pane compare for missing and changed files.
- Copy/update from compare results through the transfer queue.

## Agent Workspaces

- Create agent workspaces on any OpenDAL-backed storage.
- Workspace templates for coding, research, and data-analysis agents.
- Optional MCP policy scoping that sets default access to none and allows only the workspace root.
- Workspace manifest written through OpenDAL to `.infimount/workspace.json`.
- Visible memory files under `memory/` with read and append workflows.
- Checkpoint manifests written through OpenDAL to `.infimount/checkpoints`.
- Restore memory files from local checkpoint state or workspace checkpoint manifests.
- Workspace audit grouping for local workspace activity and MCP audit events under the workspace root.

## Release Confidence

- Release artifact builds are blocked by automated release gates.
- Frontend lint, typecheck, unit, integration, Playwright UI, and coverage gates are required.
- Rust fmt, clippy, tests, and coverage gates are required.
- Desktop smoke tests and OpenDAL storage simulator verification are required before release artifacts are built.
- Linux release artifacts receive AppImage, `.deb`, and RPM smoke/package checks before upload.
