# ADR 0003: Native Secret Storage and Plaintext Migration

**Status:** Accepted
**Date:** 2026-07-18
**Driver:** v0.8 Trust & Activation

## Context

Storage credentials (access keys, secret keys, OAuth tokens) and MCP bearer
tokens are currently persisted in plaintext JSON files under
`~/.infimount/`. Any process with filesystem access to the config directory
— or an MCP client that can read files — can exfiltrate credentials. OAuth
tokens also cross the TypeScript webview boundary, creating additional
exposure surface.

## Decision

Move all credentials to the OS-native secret store (keyring, Keychain,
Credential Manager). JSON files store only non-secret configuration and a
`secret_ref` pointer. OAuth tokens never enter the webview.

### No plaintext fallback

Stable operation MUST never fall back to plaintext secret persistence.
If the native keyring is unavailable at startup, storage operators fail
with an actionable error rather than leaking credentials to disk.

### Keyring dependency constraints

Use the current stable `keyring` crate API compatible with the workspace MSRV, Rust 1.94.
The expected API is the v4 `v1` compatibility layer. Pin the resolved
version in `Cargo.lock`. **Do not raise the project MSRV in this PR
without a separate ADR.**

## Affected files

- `crates/core/Cargo.toml`
- `crates/core/src/lib.rs`
- `crates/core/src/secrets.rs` (new)
- `crates/core/src/schema.rs`
- `crates/core/src/atomic_file.rs` (new)
- `crates/mcp/src/registry.rs`
- `crates/mcp/src/settings.rs`
- `crates/mcp/src/opendal_adapter.rs`
- `crates/mcp/src/errors.rs`
- `crates/mcp/src/tools_storage/*`
- `apps/desktop/src-tauri/src/state.rs`
- `apps/desktop/src-tauri/src/commands/storage.rs`
- `apps/desktop/src-tauri/src/commands/mcp.rs`
- `apps/desktop/src-tauri/src/commands/oauth.rs`
- `apps/desktop/src/components/AddStorageDialog.tsx`
- `apps/desktop/src/components/mcp/McpRuntimeSection.tsx`
- `apps/desktop/src/types/storage.ts`
- `apps/desktop/src/lib/api.ts`
- `docs/security.md`
- `docs/oauth-drive-setup.md`

## Contract

### Secret store API

```rust
pub enum SecretStoreStatus {
    Available,
    Locked,
    Unavailable { reason: String },
}

pub trait SecretStore: Send + Sync {
    fn status(&self) -> SecretStoreStatus;
    fn put_json(&self, account: &str, value: &serde_json::Value) -> Result<()>;
    fn get_json(&self, account: &str) -> Result<Option<serde_json::Value>>;
    fn delete(&self, account: &str) -> Result<()>;
}
```

Implementations:
- `NativeSecretStore` — OS keyring via `keyring` crate.
- `MemorySecretStore` — in-memory for tests.
- `UnavailableSecretStore` — deterministic error for tests.

### Naming convention

```
service: com.infimount.credentials
storage account: storage/<storage-uuid>
MCP HTTP account: mcp/http-auth
```

### Secret field discovery

1. `StorageFieldSchema.secret == true` from `storage_schemas.json`.
2. Conservative fallback redaction/extraction for legacy aliases and
   unknown advanced config.

### Registry migration algorithm

1. Acquire the registry lock.
2. Read the original file bytes.
3. Detect records without `schema_version == 2`.
4. If no migration is needed, release lock.
5. Create `~/.infimount/backups/storages.pre-secrets-v2.<timestamp>.json`
   with mode `0600`.
6. For each storage:
   - Extract secret values from config.
   - Store one JSON secret bundle at `storage/<id>`.
   - Remove extracted values from persisted config.
   - Set `secret_ref`, `secret_fields`, `schema_version = 2`, increment
     revision.
7. If any keyring write fails:
   - Delete keyring entries created during this migration attempt.
   - Leave the original registry untouched.
   - Return `ERR_SECRET_MIGRATION_FAILED`.
8. Atomically replace the registry.
9. Re-read the registry and assert no known secret field contains a
   non-empty plaintext value.
10. Keep the backup for rollback.

### Storage mutation behavior

- **Add:** extract secrets → write keyring → persist public record →
  roll back keyring if registry fails.
- **Edit:** load existing bundle → apply keep/set/clear → stage →
  persist registry → roll back bundle if persistence fails.
- **Remove:** remove registry record → delete keyring entry; if keyring
  fails, return warning and cleanup journal entry.
- **Operator construction:** resolve record through `SecretStore` → merge
  into in-memory config → pass to OpenDAL.

### ResolvedStorageRecord (non-serializable)

An internal type for operator construction; MUST never be serialized:

```rust
pub struct ResolvedStorageRecord {
    pub record: StorageRecord,
    pub resolved_config: serde_json::Value,
}
```

Hydrated secrets exist only in memory for the duration of operator
construction. `resolved_config` is the full config (public + secret)
passed to OpenDAL; the original `StorageRecord.config` remains
public-only.

### TypeScript interface

```ts
export type SecretMutation =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };

export interface StorageDraft {
  name: string;
  backend: StorageBackend;
  config: Record<string, unknown>;
  secretMutations: Record<string, SecretMutation>;
  oauthSessionId?: string | null;
  enabled: boolean;
  mcpExposed: boolean;
  readOnly: boolean;
}
```

Rules:
- New storage secret field with content → `set`.
- Editing existing secret defaults to `keep`.
- Clearing requires explicit `clear`.
- `********` never treated as a replacement value.
- OAuth session consumed exactly once on save.

### OAuth redesign

Current token-returning behavior is replaced with an in-memory pending
session store.

#### PendingOAuthSession

```rust
pub struct PendingOAuthSession {
    pub id: String,
    pub provider: String,
    pub secret_config: serde_json::Value,
    pub public_config: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}
```

TTL: 10 minutes from creation. Expired sessions are evicted on access.

#### Return contract

`connect_oauth_storage` returns exactly:
- `oauthSessionId`
- provider
- public non-secret fields
- expiry

It never returns access token, refresh token, client secret, authorization
code, or PKCE verifier.

#### Single-use rule

- Saving the storage consumes the pending session once and writes the
  secret bundle to the native keyring.
- Cancel, expiry, and app shutdown delete the pending session.
- Reusing a consumed session returns a deterministic error
  (`ERR_OAUTH_SESSION_ALREADY_USED`).

### MCP settings v2

Persisted JSON (`mcp_settings.json`):

```json
{
  "schemaVersion": 2,
  "enabled": false,
  "transport": "stdio",
  "bindAddress": "127.0.0.1",
  "port": 7331,
  "enabledTools": [
    "list_dir",
    "read_file",
    "read_file_version",
    "list_versions",
    "search_paths",
    "stat_path"
  ],
  "authTokenRef": null,
  "securityBaselineVersion": 2
}
```

`authToken` is never persisted. Only `authTokenRef` (a keyring pointer)
appears on disk.

#### Desktop wire model

```ts
export interface McpSettings {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
  authTokenConfigured: boolean;
}
```

#### AuthTokenMutation

```ts
export type AuthTokenMutation =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };
```

#### McpSettingsUpdate

```ts
export interface McpSettingsUpdate {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
  authTokenMutation: AuthTokenMutation;
}
```

#### Env override

The runtime may resolve `INFIMOUNT_AUTH_TOKEN` for headless HTTP use
without a configured token. This override value must never be written
to `mcp_settings.json` or any other persisted file.

### Error sanitization

Replace raw `opendal::Error::to_string()` with:

```json
{
  "kind": "PermissionDenied",
  "temporary": false,
  "operation": "read"
}
```

No URLs, query strings, headers, request bodies, or credentials.

### File permissions

Use the new `atomic_file` helper for storages, MCP settings, app settings,
workspace registry, audit log, product events, and backups.
Sensitive files: `0600`. Directories: `0700`.

## Consequences

- `storages.json` contains no plaintext secret values after migration.
- OAuth tokens never enter TypeScript responses.
- MCP bearer token absent from settings JSON.
- Grep of config directory finds no seeded secret values.
- Desktop and sidecar use the same secret store.
