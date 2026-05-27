# Infimount 0.6.0: Safe Agent Operations

Infimount 0.6.0 focuses on safer, clearer MCP access for AI agents, plus workbench keyboard polish.

Release: <https://github.com/infimount/infimount/releases/tag/v0.6.0>

## Highlights

- MCP access presets for read-only research, workspace agents, manual approval mode, and MCP lockdown.
- A clearer "What the agent can access" summary for exposed storages, enabled tools, write/destructive/link access, confirmations, active sessions, and recent risky actions.
- Audit filtering by text, decision, and storage, with copy-visible JSON and export-visible local audit bundles under `~/.infimount/exports/`.
- Redaction manifests for exported audit bundles; secrets, file contents, auth tokens, and presigned URL query strings are excluded or redacted.
- Active scoped MCP sessions are visible in desktop MCP Settings and cleared when the desktop HTTP server stops.
- Virtualized file grid and table views support roving keyboard navigation, Home/End jumps, Enter to open, and Space to toggle selection.

## Safety hardening

- Desktop HTTP now requires a bearer token when bound beyond loopback. Unauthenticated desktop HTTP is limited to local loopback development.
- MCP resources respect enabled-tool controls, preventing resource reads/lists from bypassing disabled `read_file`, `list_dir`, or `stat_path` tools.
- Recursive list, search, copy, delete, and overwrite flows enforce MCP path policy for descendant paths, so denied child prefixes cannot be exposed or mutated through an allowed parent.
- Confirmation checks validate session scope before creating approval prompts, so read-only sessions are denied deterministically.
- Cross-storage copy checks treat the source as read-like and the destination as write-like, allowing safe copies from read-only sources to writable destinations.
- Native Backblaze B2 storage drafts are accepted consistently by desktop add, update, and verify flows.

## Notes

The storage backend set remains OpenDAL-backed: local filesystem, S3/S3-compatible, native Backblaze B2, Azure Blob Storage, Google Cloud Storage, and WebDAV. Backend capabilities remain provider and configuration dependent; see the backend capability matrix for details.
