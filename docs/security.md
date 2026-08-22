# Security Model

Infimount is local-first by design. It does not require an Infimount-hosted backend to store your storage registry, runtime settings, or credentials.

## Local Data Storage

Default local files:

- `~/.infimount/storages.json`: storage registry and backend configuration.
- `~/.infimount/mcp_settings.json`: MCP runtime settings, transport, bind address, port, secret reference, and enabled tool list.

Storage credentials, OAuth tokens, and desktop MCP bearer tokens are stored in the operating system's native secret store. The v0.8 release candidate never falls back to plaintext credential persistence. Registry and settings files contain only public configuration and opaque secret references.

## Secret Handling

Infimount masks secrets in desktop control-plane storage-management outputs by default. These operations are not public MCP tools.

- Internal `list_storages` responses return masked secret values.
- Internal `export_config` responses always exclude native secret-store values; exports contain public configuration only.
- UI and MCP logs should not print raw storage config JSON or raw input payloads.
- Browser/admin-style views should replace secrets instead of revealing them by default.
- OAuth-backed storage fields such as `accessToken`, `refreshToken`, `clientSecret`, authorization codes, device codes, and PKCE verifiers are secrets and must not appear in logs, validation summaries, audit exports, or copyable diagnostic text. Guided desktop OAuth uses a local loopback callback with PKCE/state validation; tokens remain in a short-lived in-memory pending session and move to native secret storage only when the user saves the storage. See [Guided OAuth for Google Drive and Microsoft OneDrive](oauth-drive-setup.md).

## Data-Plane-Only MCP Surface

The public MCP server exposes **filesystem/data-plane tools only**. Storage administration
tools (list_storages, add_storage, edit_storage, remove_storage, import_config, export_config,
validate_storage) are not available through MCP discovery or dispatch. These functions are
reserved exclusively for the desktop control plane.

This prevents MCP clients — including AI agents — from managing storage registries or
accessing credentials through MCP. This is a pre-1.0 breaking change.

## Safe Default Tool Set

A fresh installation enables only the safe read-only tool set by default:

- list_dir
- stat_path
- read_file
- search_paths
- list_versions
- read_file_version

All write, destructive, external-link, and session tools are disabled by default. Tools are
annotated with category (Read, Write, Destructive, ExternalLink, Session) and risk level
(Low, Medium, High). Enabling any write, destructive, external-link, or session tool requires a confirmation dialog, regardless of its risk label.

Legacy settings are automatically migrated: a timestamped backup is created, the enabled tool
list is intersected with the safe default set, and the security baseline version is updated.

### Access Presets

MCP Settings provides several access presets for common configurations:

- **Read-only research**: Enables the safe read-only tool set with read-only storage access.
- **Workspace Agent**: Adds non-destructive `mkdir`, `write_file`, and `copy_path` tools while preserving existing defaults and workspace grants.
- **Manual Approval**: Enables all tools but requires explicit confirmation for every write, delete, and link operation. Safe for controlled environments where every agent mutation should be reviewed.
- **Lock down MCP**: Disables all tools and sets storage access to no access, preserving existing path rules for later restoration.

## MCP Exposure Controls

A storage is visible to MCP only when both flags are true:

- `enabled=true`
- `mcp_exposed=true`

New storages are not exposed to MCP by default from the desktop Add/Edit Storage flow or MCP storage import defaults. Exposure should be an explicit user choice.

Set `read_only=true` to prevent write, delete, move, and version-delete operations for that storage.

Tool exposure changes apply after restarting the MCP HTTP server.

## Path Policies and Confirmations

Each storage can define a local MCP policy:

- default access mode: no access, read-only, or read/write
- path rules (prefix-based, replacing the legacy allowed-paths list)
- denied path prefixes
- confirmation rules for risky operations

Path rules support both manual and workspace-managed sources. When multiple rules match, the longest normalized prefix wins. Rules with a prefix grant explicit access at a given permission level. Denied prefixes always win over allowed prefixes or rule-based access. A storage with no rules falls back to the default access mode.

Prefix matching is segment-aware and paths are normalized before policy checks so repeated slashes, trailing slashes, `.` / `..`, and URL-encoded-looking control segments such as `%2e` and `%2f` cannot bypass a deny rule. Matching remains case-sensitive because backend case behavior is not globally consistent.

Risky operations can return `requires_confirmation` instead of executing. The pending operation includes an immutable request fingerprint. Approval is valid once, expires after a bounded TTL, and cannot be replayed for a modified request. Pending approvals are in-memory runtime state and are cleared by an app/server restart.

By default, confirmations are required for:

- writes and overwrites
- deletes
- version deletes
- presigned/download-link generation
- cross-storage copy
- rename/move operations that may behave like copy plus delete

Desktop notifications, when enabled by the user, are attention signals only. They do not approve or deny operations and do not include tokens, secrets, or presigned URLs.

## Dependency audit policy

The scheduled dependency audit fails on high-severity advisories. The only current RustSec exception is `RUSTSEC-2023-0071` (Marvin timing attack in `rsa`), which has no fixed release and is pulled transitively by reqsign's optional Azure client-certificate/JWT support. Infimount does not enable or expose that credential mode. The exact exception is passed to `cargo audit` in `.github/workflows/deps-audit.yml` and must be removed when a fixed `rsa` release is available. JavaScript production audits have no ignored advisories.

## MCP Audit Log

Infimount stores a bounded local MCP audit log at `~/.infimount/mcp_audit.json`.

Audit events include tool name, storage metadata when available, operation, path, decision, matched rule ID, workspace ID, confirmation ID, duration, and error code. The audit log records allowed, denied, confirmation-required, confirmed, and failed operations.

Safety rules:

- auth tokens are not logged
- storage secrets are not logged
- file contents are not logged
- presigned URLs have query strings redacted before persistence and export
- sensitive headers are not logged

The desktop audit viewer can export the current filtered rows as a local JSON bundle under `~/.infimount/exports/`. The bundle includes a redaction manifest stating that secrets, file contents, auth tokens, and presigned URL query strings are excluded or redacted.

## HTTP Transport

For desktop and local development, keep HTTP bound to loopback:

```text
127.0.0.1
```

Desktop HTTP can run unauthenticated only on loopback for local development. If you bind desktop HTTP to `0.0.0.0` or a LAN address, Infimount requires a bearer token before the server can start.

Headless HTTP mode also requires bearer-token authentication unless explicitly started with `--allow-insecure` on loopback. Set a token with either CLI or environment:

```bash
INFIMOUNT_AUTH_TOKEN='replace-with-a-random-token' infimount_mcp --transport http --bind 127.0.0.1 --port 7331
```

Clients must send:

```text
Authorization: Bearer replace-with-a-random-token
```

Only bind to `0.0.0.0` or a LAN address when you intentionally expose the server and have a strong token plus a network boundary in place. Desktop bearer tokens are stored in native secret storage; generated snippets use an `INFIMOUNT_AUTH_TOKEN` placeholder and never reveal the stored value. If native secret storage is unavailable or locked, Infimount fails with an actionable error instead of persisting or using plaintext fallback credentials.

## Sessions and Scoped Access

MCP clients can create scoped sessions with:

- allowed storage names
- optional allowed path prefixes
- optional read-only override
- TTL

When a session is created without the read-only override, each requested prefix is validated for writable access through the storage policy. If a prefix only has read access at the policy level, the session creation is rejected with a clear error. Filesystem tools that receive a `session_id` enforce those restrictions before backend operations. Session path prefixes are normalized and segment-aware to avoid broad matches such as allowing `docs2` when only `docs` was scoped. Desktop MCP Settings shows active in-memory scoped sessions with their storage scope, path prefixes, read-only status, and expiry so users can inspect current agent scopes. Desktop HTTP sessions are cleared when that server stops.

## Storage Validation Safety

The desktop **Validate** action reports reachability, effective capabilities, sanitized fix hints, and MCP readiness notes. Storage validation is desktop-only and is not exposed as a public MCP tool. Validation output must not include raw credentials, auth tokens, storage config JSON, or file contents. MCP readiness notes are advisory only; users must still explicitly enable storage exposure, tool access, path policy, read-only mode, and confirmations.

## Backend Capability Boundaries

Some capabilities are backend-dependent:

- Object versions require backend and bucket/container support.
- Presigned download links require backend support.
- Google Drive, WebDAV, SFTP, and FTP do not expose object-version tools. OneDrive version tools require `versioning` to be enabled and supported by the account.

See [Backend Capability Matrix](backend-capabilities.md) for the public support matrix.

## Bounded MCP Version Listing

`list_versions` is enabled in the safe default tool set because the implementation is bounded:

- at most **10,000** versions are scanned per path;
- at most **1,000** versions are returned per page;
- results are sorted only from the bounded collected set;
- a `truncated: true` flag is returned when more versions may exist;
- pagination continues through an HMAC-signed cursor bound to the storage, the normalized path, and the storage revision.

The cursor cannot be replayed on another path or storage, cannot survive a storage revision change, and never returns a continuation beyond the scan cap. Desktop and MCP share this single bounded implementation.

## Upload Staging

Desktop uploads are staged in a per-user application-owned directory before the destination is created:

```text
$XDG_RUNTIME_DIR/infimount/upload-staging
```

with fallback to the per-user application cache directory. The staging root is verified with `symlink_metadata`, rejects symlinks and non-directories, and on Unix requires the directory owner to match the current effective UID with mode `0700`. Staged files use UUID names created with `create_new` and mode `0600`, and stale cleanup traverses only the verified owned root without following symlinks. A cross-process file lock enforces an aggregate staging quota across processes.

## Bounded and Atomic MCP `write_file`

`write_file` accepts at most **4 MiB** of text content and is disabled by default. A no-overwrite write uses the backend's atomic create-if-absent operation; a backend that cannot guarantee atomic no-overwrite returns `ERR_BACKEND_UNSUPPORTED` rather than risking a stat-then-write race that could silently overwrite an existing object.

## Transactional Directory Transfers

Directory transfers that create a previously absent destination are transactional: the tree is staged and committed with a rename where the backend supports it. On failure or cancellation the transaction-created destination is removed; a destination that existed before the operation is never deleted. If cleanup itself fails, the error reports `partialDestination: true` and `cleanupRequired: true` without exposing local absolute roots or credentials.


## Local-filesystem MCP confinement

Before every policy-authorized local MCP operation, Infimount rejects existing
symlink and Windows reparse-point components beneath the configured storage
root. This prevents persistent project links from redirecting an agent outside
the approved namespace. The current RC threat model assumes another trusted
local process does not replace path components during the brief operation
window; handle-relative, race-free confinement remains planned hardening.

## HTTP transport boundary

The built-in Streamable HTTP transport is loopback-only. Remote access requires
a TLS-terminating authenticated reverse proxy. A bearer token alone is not
treated as transport encryption.
