# Roadmap: v0.5 to v0.7

Infimount remains OpenDAL-first. File operations should route through OpenDAL-backed Rust core/MCP layers, not provider-specific SDK paths.

## v0.5: Workbench Reliability

Theme: search, compare, dry-run, and recover.

Planned work:

1. Global activity log foundation for transfers, writes, deletes, conflict choices, dry-runs, recovery attempts, and MCP operations.
2. Transfer manifests that describe intended and completed file-level work.
3. Operation dry-run for copy, move, delete, overwrite, skip, and keep-both decisions.
4. Interrupted transfer recovery at file granularity: retry incomplete files, skip completed files, verify with metadata where OpenDAL exposes it.
5. Opt-in local path/name/metadata index per storage.
6. Global search across indexed storages with index freshness shown.
7. Dual-pane compare using path/name/size/modified metadata first.
8. Copy/update from compare results through dry-run and transfer manifests.

Implementation status:

- Transfer queue and persisted transfer history are in place.
- Core transfer dry-run planning is implemented with an OpenDAL-only `plan_transfer_entries` manifest API.
- Transfer queue jobs store dry-run summaries and write local activity-log events for planned, started, completed, failed, cancelled, and recovery-started transfers.
- Interrupted transfers are restored as failed/retryable and recovery retries use completed-file skip behavior when safe.
- Recursive OpenDAL metadata scans support opt-in local path/name/metadata indexing.
- Global search across indexed storages is available from the storage menu.
- Dual-pane compare can detect missing/changed files and copy/update from the compare result through the transfer queue.

## v0.6: Agent Workspaces

Theme: make safe MCP storage scopes a first-class product concept.

Planned work:

1. Create an agent workspace on any OpenDAL-backed storage.
2. Workspace-scoped MCP access and policy.
3. Workspace templates for coding, research, and data analysis agents.
4. Visible workspace memory files for append, read, and list workflows.
5. Workspace checkpoint and restore where OpenDAL versioning or transfer manifests support it.
6. Audit events grouped by workspace.

## v0.7: OpenDAL Backend Expansion

Theme: broaden storage coverage without leaving the OpenDAL abstraction.

Candidate additions:

- SFTP.
- SMB if OpenDAL support is adequate.
- HDFS if useful for data users.
- OSS, COS, OBS, and similar object stores through OpenDAL.
- More WebDAV presets.

Rule for every backend:

```text
new backend = OpenDAL operator + capability matrix + tests + docs
```

No direct provider SDKs for file operations.
