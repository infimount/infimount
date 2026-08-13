# Infimount v0.8 — Trust & Activation
## Execution-grade implementation plan for coding agents

**Target repository:** `infimount/infimount`
**Target release:** `v0.8.0`
**Planning baseline:** `main` as reviewed in July 2026. Before coding, record the actual HEAD SHA and re-check every referenced file.
**Primary outcome:** A clean-machine user can install Infimount, create or select a scoped workspace, connect a local MCP client through the bundled sidecar, complete a verified read, and see proof that access outside the workspace is denied.

---

# 1. Agent execution contract

This document is the implementation source of truth for v0.8. A coding agent must follow these rules:

1. Implement work in the PR order defined in section 6. Do not combine unrelated PRs.
2. Run the complete validation commands for each PR before moving to the next PR.
3. Preserve current public APIs unless this document explicitly declares a breaking change.
4. Treat any value identified as a credential, OAuth token, bearer token, password, private key, authorization code, PKCE verifier, or provider secret as sensitive.
5. Never log, serialize into error details, include in diagnostics, copy to clipboard, emit to the webview, or persist in plaintext any sensitive value.
6. Do not add storage backends, mobile work, hosted sync, OS filesystem mounting, or generic MCP marketplace work in v0.8.
7. Do not silently broaden MCP access during migration.
8. Do not use `unwrap`, `expect`, or panic for user-controlled input, config files, sidecar discovery, provider responses, or storage operations. Tests may use them.
9. Every persisted format introduced or changed in v0.8 must include a schema version and a migration test.
10. Every file mutation must be atomic and must create a recoverable backup where this document requires one.
11. New network telemetry must be disabled by default and must never include names, paths, file contents, prompts, credentials, or provider identifiers beyond the coarse backend type.
12. When implementation details conflict with this plan, stop and create an ADR before changing the contract.

---

# 2. Release scope

## 2.1 Required v0.8 deliverables

v0.8 must include all of the following:

- MCP data-plane-only public tool surface.
- Read-only safe default tool profile.
- Policy schema v2 with explicit per-prefix grants.
- Correct Agent Workspace grants, including multiple workspaces on one storage.
- Native OS credential storage and plaintext-secret migration.
- No secret-inclusive export over MCP.
- Safe shareable export, encrypted recovery backup, import preview, and atomic apply.
- Bundled `infimount_mcp` sidecar in all desktop installers.
- Desktop and sidecar version synchronization.
- Workspace metadata persisted by Rust, not browser `localStorage`.
- Workspace-first activation wizard.
- Client setup previews and supported client installers.
- A real stdio MCP protocol probe.
- Sanitized diagnostics and local opt-in product events.
- Operator caching and large-directory listing foundation.
- Range-based preview reads and streaming transfers.
- Tauri hardening and stable-release signing enforcement.
- Upgrade, rollback, security, client setup, and release documentation.

## 2.2 Explicit non-goals

Do not implement these in v0.8:

- New storage providers.
- Mobile applications.
- Hosted Infimount accounts or sync.
- Multi-user cloud policy management.
- OS-level Finder/Explorer mounts.
- A remotely exposed administrative MCP server.
- Public OAuth client ownership for Google Drive or OneDrive.
- An in-house cryptographic keyring replacement.
- Automatic editing of arbitrary JSONC files when comments cannot be preserved safely.
- Automatic enabling of write or destructive tools during onboarding.

---

# 3. Non-negotiable product and security invariants

The implementation is incomplete if any invariant below is false.

## 3.1 MCP surface

- The public MCP server exposes filesystem/data-plane tools only.
- Storage registry administration is available to trusted desktop commands, not normal MCP clients.
- The default enabled tools are exactly:

```text
list_dir
stat_path
read_file
search_paths
list_versions
read_file_version
```

- The following tools are disabled by default:

```text
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

- These storage administration tools are removed from `tool_definitions()`, MCP discovery, and MCP dispatch:

```text
list_storages
add_storage
edit_storage
remove_storage
import_config
export_config
validate_storage
```

Desktop commands may continue to call the underlying Rust functions directly.

## 3.2 Storage exposure

- `StorageRecord::new()` defaults to `mcp_exposed = false`.
- A new storage policy grants no access until the user explicitly chooses whole-storage access or creates a path/workspace grant.
- An Agent Workspace defaults to read-only.
- A workspace root may not be empty or `/`.
- A workspace grant never replaces unrelated workspace grants on the same storage.
- Denied prefixes always override grants.
- Sessions can only reduce access; they cannot expand storage policy access.

## 3.3 Secrets

- `storages.json` contains no plaintext secret values after successful migration.
- `mcp_settings.json` contains no plaintext bearer token.
- OAuth tokens are never returned to TypeScript.
- MCP responses never expose credential material.
- Shareable exports contain no secret values and no usable secret-store references.
- Diagnostics and audit exports contain no secrets.
- Stable operation never falls back to plaintext secret persistence.

## 3.4 Installation

- A desktop installer includes a same-version `infimount_mcp` executable.
- Generated stdio configuration uses a verified absolute executable path.
- A user does not need Rust, Node.js, Cargo, pnpm, or a separate MCP installation.
- Sidecar absence or version mismatch produces a specific, actionable error.

## 3.5 Activation

Onboarding is complete only when all conditions are true:

1. A storage or demo storage exists.
2. A workspace exists.
3. A safe policy grants the workspace and denies an outside fixture.
4. The bundled sidecar passes a version/self-check.
5. A stdio MCP initialize request succeeds.
6. `list_dir` succeeds for the workspace.
7. `read_file` succeeds for a fixture inside the workspace.
8. `read_file` fails with policy denial for a fixture outside the workspace.
9. The result is recorded in local activation state.

---

# 4. Target architecture

```text
┌───────────────────────────────────────────────────────────────┐
│ Tauri desktop control plane                                   │
│                                                               │
│ Storage setup │ Keyring │ Workspace setup │ Policies           │
│ Client setup  │ Approvals │ Audit │ Diagnostics │ Backups       │
└──────────────────────┬────────────────────────────────────────┘
                       │ trusted internal Rust APIs
                       ▼
┌───────────────────────────────────────────────────────────────┐
│ Shared local runtime                                           │
│                                                               │
│ StorageRegistry │ SecretStore │ WorkspaceRegistry              │
│ PolicyEvaluator │ OperatorCache │ AuditStore                   │
└──────────────────────┬────────────────────────────────────────┘
                       │ restricted data-plane context
                       ▼
┌───────────────────────────────────────────────────────────────┐
│ Bundled infimount_mcp sidecar                                  │
│                                                               │
│ stdio / loopback HTTP │ safe tool set │ policy enforcement     │
│ confirmations │ scoped sessions │ bounded output │ local audit │
└───────────────────────────────────────────────────────────────┘
```

The desktop control plane may perform storage administration. The MCP sidecar must not.

---

# 5. Required data contracts

## 5.1 Tool metadata

Extend `ToolDefinition` in `crates/mcp/src/server.rs`:

```rust
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

Do not add an `Administration` category to the public data-plane server. Administrative functions remain internal desktop APIs.

## 5.2 Policy schema v2

Replace the current allow-list-as-restriction-only model with explicit path rules.

```rust
pub const MCP_POLICY_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpRuleSource {
    Manual,
    Workspace { workspace_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPathRule {
    pub id: String,
    pub prefix: String,
    pub access: McpAccessMode,
    pub source: McpRuleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_rules: Option<McpConfirmationRules>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpStoragePolicy {
    pub version: u32,
    pub default_access: McpAccessMode,
    pub rules: Vec<McpPathRule>,
    pub denied_paths: Vec<String>,
    pub confirmation_rules: McpConfirmationRules,

    // Deserialize legacy policies only. Never emit this field after migration.
    #[serde(default, skip_serializing)]
    pub allowed_paths: Vec<String>,
}
```

New default:

```rust
McpStoragePolicy {
    version: 2,
    default_access: McpAccessMode::None,
    rules: vec![],
    denied_paths: vec![],
    confirmation_rules: McpConfirmationRules::default(),
    allowed_paths: vec![],
}
```

### Policy evaluation algorithm

`evaluate_storage_policy()` must execute in this exact order:

1. Reject when `storage.mcp_exposed == false`.
2. Normalize the requested backend path.
3. Reject if any `denied_paths` prefix matches.
4. Find all matching `rules`; choose the rule with the longest normalized prefix.
5. Reject duplicate rules with the same normalized prefix during normalization. Do not resolve ambiguity at request time.
6. Effective access is the matched rule’s access; otherwise it is `default_access`.
7. If `storage.read_only` is true, any write-like operation is rejected regardless of the rule.
8. If effective access is `none`, reject.
9. If effective access is `read_only` and the operation is write-like, reject.
10. Use the matched rule’s confirmation rules when present; otherwise use policy-level confirmation rules.
11. Return the decision plus `matched_rule_id` and optional `workspace_id` for audit.

Required return model:

```rust
pub struct PolicyEvaluation {
    pub decision: PolicyDecision,
    pub matched_rule_id: Option<String>,
    pub workspace_id: Option<String>,
}
```

### Legacy policy migration

When loading a policy without `version == 2`:

- Normalize all legacy `allowed_paths`.
- When `allowed_paths` is non-empty, create one manual rule per prefix.
- Rule access is:
  - legacy `read_write` -> `read_write`
  - legacy `read_only` -> `read_only`
  - legacy `none` -> `read_only` as a safety repair for the existing workspace-policy bug
- Set `default_access = none`.
- Preserve `denied_paths` and confirmation rules.
- Set `version = 2`.
- Save the migrated registry atomically.
- Create a pre-migration backup first.

When legacy `allowed_paths` is empty, preserve the legacy default access.

## 5.3 Persisted storage record

Keep existing snake_case field naming for compatibility. Add:

```rust
pub const STORAGE_RECORD_SCHEMA_VERSION: u32 = 2;

pub struct StorageRecord {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub backend: String,

    // Public/non-secret config only.
    pub config: serde_json::Value,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,

    #[serde(default)]
    pub secret_fields: Vec<String>,

    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
    pub mcp_policy: McpStoragePolicy,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}
```

`secret_fields` contains top-level field names or JSON pointers if nested secret config is introduced. Use one representation consistently; JSON Pointer is preferred.

Do not serialize hydrated secrets into `StorageRecord`.

Create an internal non-serializable type:

```rust
pub struct ResolvedStorageRecord {
    pub record: StorageRecord,
    pub resolved_config: serde_json::Value,
}
```

## 5.4 Secret mutation contract

TypeScript and Rust must use explicit secret mutations.

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

- New storage secret field with content -> `set`.
- Editing an existing secret field defaults to `keep`.
- Clearing requires an explicit `clear`.
- The literal mask `********` is never treated as a replacement value.
- An OAuth session is consumed exactly once when the storage is saved.

## 5.5 MCP settings v2

Persist:

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

Do not persist `authToken`.

Desktop wire model:

```ts
export interface McpSettings {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
  authTokenConfigured: boolean;
}

export type AuthTokenMutation =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };
```

Update request:

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

The runtime may resolve an environment override `INFIMOUNT_AUTH_TOKEN` for headless HTTP, but it must not write that value to disk.

## 5.6 Workspace record

Persist workspaces under the Infimount config directory, not browser storage.

```rust
pub const WORKSPACE_SCHEMA_VERSION: u32 = 1;

pub enum WorkspaceAccessProfile {
    ReadOnly,
    ReadWriteConfirmed,
}

pub struct WorkspaceRecord {
    pub schema_version: u32,
    pub id: String,
    pub storage_id: String,
    pub name: String,

    // Normalized backend path: no leading/trailing slash and never empty.
    pub root_path: String,

    pub template_id: String,
    pub access_profile: WorkspaceAccessProfile,
    pub policy_rule_id: String,
    pub memory_files: Vec<String>,
    pub checkpoint_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

Path examples:

```text
Persisted: agent-workspaces/coding-workspace
Displayed: /agent-workspaces/coding-workspace
MCP path: /StorageName/agent-workspaces/coding-workspace
```

## 5.7 Activation state

Extend `AppSettings`:

```rust
pub const ONBOARDING_VERSION: u32 = 2;

pub enum TelemetryConsent {
    Unknown,
    Granted,
    Denied,
}

pub struct ActivationState {
    pub current_step: String,
    pub demo_storage_id: Option<String>,
    pub workspace_id: Option<String>,
    pub selected_client: Option<String>,
    pub sidecar_verified_at: Option<String>,
    pub storage_validated_at: Option<String>,
    pub first_mcp_read_at: Option<String>,
    pub outside_scope_denial_verified_at: Option<String>,
    pub completed_at: Option<String>,
}

pub struct AppSettings {
    pub onboarding_version: u32,
    pub onboarding_completed: bool,
    pub onboarding_skipped: bool,
    pub activation: ActivationState,
    pub telemetry_consent: TelemetryConsent,
    pub security_review_version: u32,
    // Retain current timestamp fields.
}
```

---

# 6. Ordered PR implementation plan

## PR 00 — Baseline, ADRs, and module boundaries

### Goal

Create a stable implementation baseline and reduce merge conflicts before behavior changes.

### Files to add

```text
docs/roadmaps/v0.8-trust-activation.md
docs/adr/0001-mcp-data-plane-only.md
docs/adr/0002-policy-v2.md
docs/adr/0003-native-secret-storage.md
docs/adr/0004-bundled-mcp-sidecar.md
scripts/v08-baseline.sh
```

### Refactor without behavior change

Split `apps/desktop/src-tauri/src/commands.rs` into:

```text
apps/desktop/src-tauri/src/commands/mod.rs
apps/desktop/src-tauri/src/commands/storage.rs
apps/desktop/src-tauri/src/commands/mcp.rs
apps/desktop/src-tauri/src/commands/oauth.rs
apps/desktop/src-tauri/src/commands/transfers.rs
apps/desktop/src-tauri/src/commands/settings.rs
```

Do not change command names or argument serialization in this PR.

Split `McpSettingsDialog.tsx` only if tests already cover it. Preferred target:

```text
apps/desktop/src/components/mcp/McpSettingsDialog.tsx
apps/desktop/src/components/mcp/McpRuntimeSection.tsx
apps/desktop/src/components/mcp/McpToolSection.tsx
apps/desktop/src/components/mcp/McpPolicySection.tsx
apps/desktop/src/components/mcp/McpAuditSection.tsx
```

### Baseline script

`scripts/v08-baseline.sh` must run:

```bash
set -euo pipefail
pnpm install --frozen-lockfile
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop test:integration
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings \
  -A clippy::result_large_err \
  -A clippy::needless_borrows_for_generic_args
cargo test --workspace
```

### Acceptance criteria

- No functional diff.
- Existing tests remain green.
- ADRs contain the contracts in this document.
- Actual baseline HEAD SHA is recorded in the roadmap.

---

## PR 01 — Data-plane-only MCP and safe tool defaults

### Goal

Remove storage administration from normal MCP clients and make a safe read-only tool set the default.

### Primary files

```text
crates/mcp/src/server.rs
crates/mcp/src/settings.rs
crates/mcp/src/schemas.rs
crates/mcp/src/lib.rs
crates/mcp/src/tools_storage/*
apps/desktop/src/components/mcp/McpToolSection.tsx
apps/desktop/src/components/mcp/McpSettingsDialog.tsx
apps/desktop/src/types/storage.ts
apps/desktop/src/pages/Index.tsx
docs/mcp-client-setup.md
docs/security.md
```

### Required changes

1. Add tool category/risk/default metadata.
2. Remove all storage-administration tools from:
   - `tool_definitions()`
   - `dispatch_tool_json()`
   - MCP schemas reachable by public discovery
   - helper invoke functions intended only for MCP dispatch
3. Keep `tools_storage` Rust functions callable by desktop commands.
4. Change `default_enabled_tool_names()` to the exact safe set in section 3.1.
5. Add `security_baseline_version` to settings.
6. Migrate legacy settings:
   - Before mutation, copy `mcp_settings.json` to `backups/mcp_settings.pre-v0.8.<timestamp>.json`.
   - If `securityBaselineVersion` is missing or less than 2, replace `enabledTools` with the intersection of the old list and the safe default set.
   - If the intersection is empty, use the complete safe default set.
   - Set version 2 and persist.
7. Remove the one-click `Enable all` action. Replace it with:
   - `Apply safe read-only`
   - `Configure advanced tools`
8. Advanced tool enabling:
   - Group tools by category.
   - Show medium/high-risk labels.
   - Enabling a write, destructive, external-link, or session tool requires a confirmation dialog.
9. Correct frontend HTTP-token plumbing temporarily:
   - `McpSettingsWire` includes `authToken`.
   - `mapStatusWire()` maps it.
   - `handleSaveMcpSettings()` sends it.
   - PR 03 removes raw token transport entirely.
10. Update tool annotations:
    - read tools: read-only, non-destructive, idempotent.
    - `generate_download_link`: read-only, non-destructive, idempotent, open-world.
    - write tools: not read-only.
    - move/delete/delete-version: destructive.
    - session tools: non-destructive but not default-enabled.

### Required tests

Rust:

- Default settings contain only the safe set.
- Legacy settings with every tool migrate to the safe set.
- Admin tool names are rejected during settings normalization.
- Admin tools do not appear in `list_tools`.
- Calling a removed admin tool returns method-not-found.
- Desktop-internal storage administration functions still work.

Frontend:

- Safe profile is displayed by default.
- No `Enable all` button exists.
- Enabling a high-risk tool requires confirmation.
- Cancelling confirmation leaves the tool disabled.
- HTTP auth token survives save/reload during this interim PR.

### Acceptance criteria

- An MCP client cannot discover or invoke registry administration.
- A fresh installation exposes no write/destructive tool by default.
- Existing settings are safely migrated with a backup.
- Documentation marks the MCP admin-tool removal as a pre-1.0 breaking change.

---

## PR 02 — Policy v2 and correct workspace grants

### Goal

Create explicit, composable path grants and repair the current Agent Workspace access semantics.

### Primary files

```text
crates/mcp/src/policy.rs
crates/mcp/src/registry.rs
crates/mcp/src/tools_fs/common.rs
crates/mcp/src/server.rs
crates/mcp/src/session.rs
crates/mcp/src/audit.rs
apps/desktop/src/types/storage.ts
apps/desktop/src/pages/Index.tsx
apps/desktop/src/components/mcp/McpPolicySection.tsx
apps/desktop/src/lib/agentWorkspaces.ts
docs/security.md
docs/mcp-client-setup.md
```

### Required changes

1. Implement the policy v2 structs and evaluation algorithm from section 5.2.
2. Add centralized functions:
   - `normalize_policy_path`
   - `normalize_policy_rule`
   - `normalize_storage_policy`
   - `migrate_legacy_policy`
3. Reject duplicate normalized rule prefixes.
4. Use longest-prefix match.
5. Add optional `matched_rule_id` and `workspace_id` to audit events.
6. Extend audit filtering and UI to display workspace when available.
7. Change `StorageRecord::new()` to:
   - `mcp_exposed = false`
   - policy v2 default with no access
8. Change frontend fallback policy from `read_write` to policy v2 no-access.
9. Change workspace policy creation:
   - Never set `default_access = none` while relying on legacy `allowed_paths`.
   - Add or update one `McpPathRule` with source `workspace`.
   - Default access profile is read-only.
   - Preserve other rules.
10. Prohibit workspace root `/`.
11. Session creation validation:
   - Requested storage must be enabled and exposed.
   - Requested prefix must be inside an effective policy grant.
   - A writable session cannot elevate a read-only grant.
   - A session without prefixes still remains bounded by storage policy.
12. Presets:
   - `Read-only research`: safe tool set; convert existing non-deny grants to read-only.
   - `Workspace agent`: enable non-destructive write tools only; preserve each workspace’s selected access profile.
   - `Manual approval`: enable broad data-plane tools; preserve path grants; confirmation rules remain on.
   - `Lock down`: empty tool set and set whole-storage default to none; do not delete workspace rules, but mark them inactive through tools being disabled.

### Required migration tests

Fixtures must cover:

- Legacy read-write policy with empty `allowed_paths`.
- Legacy read-only policy with allowed path.
- Legacy none policy with allowed path created by the old workspace builder.
- Repeated slashes, `..`, `%2e`, `%2f`, backslashes, and trailing slashes.
- Two workspaces on the same storage.
- Nested rules where the longer prefix wins.
- A global denied prefix overriding a workspace grant.

### Required behavioral tests

- Read-only workspace allows read inside root.
- Read-only workspace denies write inside root.
- Workspace denies read outside root.
- Read-write-confirmed workspace returns confirmation requirement on write.
- Creating a second workspace does not remove the first rule.
- Removing a workspace rule does not change unrelated rules.
- Session cannot elevate access.
- Root workspace is rejected.

### Acceptance criteria

- Agent Workspaces are actually accessible under their intended roots.
- Multiple workspace grants coexist.
- No path grant can silently expand access beyond its prefix.
- Every allowed/denied decision can identify the matched rule in audit.

---

## PR 03 — Native secret storage and plaintext migration

### Goal

Move storage credentials and MCP bearer tokens out of JSON files and prevent tokens from crossing the webview.

### Dependencies

Use the current stable `keyring` crate API compatible with Rust 1.85. The expected API is the v4 `v1` compatibility layer. Pin the resolved version in `Cargo.lock`. Do not raise the project MSRV in this PR without a separate ADR.

### Primary files

```text
crates/core/Cargo.toml
crates/core/src/lib.rs
crates/core/src/secrets.rs                # new
crates/core/src/schema.rs
crates/core/src/atomic_file.rs             # new
crates/mcp/src/registry.rs
crates/mcp/src/settings.rs
crates/mcp/src/opendal_adapter.rs
crates/mcp/src/errors.rs
crates/mcp/src/tools_storage/*
apps/desktop/src-tauri/src/state.rs
apps/desktop/src-tauri/src/commands/storage.rs
apps/desktop/src-tauri/src/commands/mcp.rs
apps/desktop/src-tauri/src/commands/oauth.rs
apps/desktop/src/components/AddStorageDialog.tsx
apps/desktop/src/components/mcp/McpRuntimeSection.tsx
apps/desktop/src/types/storage.ts
apps/desktop/src/lib/api.ts
docs/security.md
docs/oauth-drive-setup.md
```

### Secret store API

Create:

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

Implement:

- `NativeSecretStore`
- `MemorySecretStore` for tests
- `UnavailableSecretStore` for deterministic error tests

Naming:

```text
service: com.infimount.credentials
storage account: storage/<storage-uuid>
MCP HTTP account: mcp/http-auth
```

### Secret field discovery

Use both:

1. `StorageFieldSchema.secret == true` from `crates/core/storage_schemas.json`.
2. A conservative fallback redaction/extraction matcher for legacy aliases and unknown advanced config.

Schema flags are authoritative for known fields. The fallback may move a non-secret identifier into keyring; that is acceptable. It must never leave a probable secret in plaintext.

### Registry migration algorithm

Run migration before normal registry use.

1. Acquire the registry lock.
2. Read the original file bytes.
3. Detect records without `schema_version == 2`.
4. If no migration is needed, release lock.
5. Create `~/.infimount/backups/storages.pre-secrets-v2.<timestamp>.json` with mode `0600` on Unix.
6. For each storage:
   - Extract secret values from config.
   - Store one JSON secret bundle at `storage/<id>`.
   - Remove extracted values from persisted config.
   - Set `secret_ref`, `secret_fields`, `schema_version = 2`, and increment revision.
7. If any keyring write fails:
   - Delete keyring entries created during this migration attempt.
   - Leave the original registry untouched.
   - Return `ERR_SECRET_MIGRATION_FAILED`.
8. Atomically replace the registry.
9. Re-read the registry and assert no known secret field contains a non-empty plaintext value.
10. Keep the backup for rollback.

### Storage mutation behavior

- Add storage:
  - Extract secret mutations.
  - Write secret bundle.
  - Persist public record.
  - On registry failure, delete the newly written secret.
- Edit storage:
  - Load existing bundle.
  - Apply explicit keep/set/clear mutations.
  - Stage updated bundle.
  - Persist registry with revision increment.
  - Roll back bundle if registry persistence fails.
- Remove storage:
  - Remove registry record atomically.
  - Delete keyring entry after successful registry mutation.
  - If keyring deletion fails, return a warning and add a cleanup journal entry; do not resurrect the registry record.
- Operator construction:
  - Resolve the stored record through `SecretStore`.
  - Merge secret bundle into an in-memory config.
  - Pass only the in-memory resolved config to OpenDAL.

### OAuth redesign

Current token-returning behavior must be replaced.

Add an in-memory `PendingOAuthStore` to `AppState`:

```rust
pub struct PendingOAuthSession {
    pub id: String,
    pub provider: String,
    pub secret_config: serde_json::Value,
    pub public_config: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}
```

Rules:

- `connect_oauth_storage` returns:
  - `oauthSessionId`
  - provider
  - public non-secret fields
  - expiry
- It never returns access token, refresh token, client secret, code, or verifier.
- Session TTL: 10 minutes.
- Saving the storage consumes the session once and writes secrets to keyring.
- Cancel, expiry, and app shutdown delete the pending session.
- Reusing a consumed session returns a deterministic error.

### MCP auth-token redesign

- Persist `auth_token_ref`, not the token.
- Runtime status returns `authTokenConfigured`.
- Settings updates use explicit keep/set/clear.
- HTTP snippets never contain the raw token.
- For advanced HTTP usage, show an environment-variable placeholder.
- Headless CLI may resolve `INFIMOUNT_AUTH_TOKEN`; it never persists the value.

### Error sanitization

Replace raw `opendal::Error::to_string()` details with sanitized fields:

```json
{
  "kind": "PermissionDenied",
  "temporary": false,
  "operation": "read"
}
```

Do not include URLs, query strings, headers, request bodies, credentials, or provider messages that may echo secrets.

### File permissions

Use the new atomic-file helper for:

```text
storages.json
mcp_settings.json
app_settings.json
workspace registry
audit log
product events
backups
```

On Unix, create sensitive files as `0600` and directories as `0700`.

### Required tests

- Fresh storage credentials never appear in `storages.json`.
- Legacy credentials migrate and still construct a valid mocked operator.
- Migration failure preserves original file.
- Edit with `keep` preserves secret.
- Edit with `clear` removes secret.
- Literal `********` never replaces a secret.
- OAuth response contains no token fields.
- Pending OAuth session is single-use and expires.
- MCP auth token is absent from settings JSON and runtime-status serialization.
- Keyring unavailable produces actionable error.
- Logs, errors, audit, diagnostics, and exports contain none of a seeded secret corpus.
- Concurrent access to one secret entry is serialized.

### Acceptance criteria

- Grep of the config directory after tests finds none of the seeded secret values.
- Desktop can still connect to mocked S3, Drive, and OneDrive through resolved credentials.
- Sidecar and desktop use the same secret store.

---

## PR 04 — Shareable export, encrypted recovery, and safe import

### Goal

Replace secret-inclusive raw exports and destructive blind imports.

### Dependencies

Use a mature passphrase-encryption library such as `age`. Pin the resolved dependency. Do not implement custom encryption primitives.

### Primary files

```text
crates/core/src/backup.rs                 # new
crates/core/src/lib.rs
crates/mcp/src/tools_storage/export_config.rs
crates/mcp/src/tools_storage/import_config.rs
apps/desktop/src-tauri/src/commands/backup.rs   # new
apps/desktop/src-tauri/src/commands/storage.rs
apps/desktop/src/lib/api.ts
apps/desktop/src/pages/Index.tsx
apps/desktop/src/components/StorageConfigEditorDialog.tsx
apps/desktop/src/components/StorageImportDialog.tsx   # new
apps/desktop/src/components/RecoveryBackupDialog.tsx  # new
apps/desktop/src/types/storage.ts
docs/security.md
docs/recovery.md                         # new
```

### Breaking API changes

Remove:

```ts
exportStorageConfig(includeSecrets: boolean)
```

Add:

```ts
exportShareableConfig(): Promise<ShareableExportResult>
previewStorageImport(json: string): Promise<StorageImportPreview>
applyStorageImport(request: ApplyStorageImportRequest): Promise<ApplyStorageImportResult>
createRecoveryBackup(request: RecoveryBackupRequest): Promise<RecoveryBackupResult>
previewRecoveryRestore(request: RecoveryRestorePreviewRequest): Promise<StorageImportPreview>
applyRecoveryRestore(request: ApplyRecoveryRestoreRequest): Promise<ApplyStorageImportResult>
```

### Shareable export format

```json
{
  "schemaVersion": 2,
  "kind": "infimount-shareable-config",
  "exportedAt": "RFC3339",
  "storages": [
    {
      "name": "Docs",
      "backend": "s3",
      "config": {
        "bucket": "example",
        "region": "us-east-1"
      },
      "requiredSecretFields": [
        "/accessKeyId",
        "/secretAccessKey"
      ],
      "enabled": true,
      "mcpExposed": false,
      "readOnly": false,
      "mcpPolicy": {
        "version": 2,
        "default_access": "none",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {}
      }
    }
  ]
}
```

Rules:

- Never include `secret_ref`.
- Never include storage UUIDs unless required for an internal recovery bundle.
- Force `mcpExposed` false in shareable export.
- Imported shareable config remains unexposed until the user explicitly enables it.
- Mark missing secret fields for re-entry.

### Encrypted recovery bundle

The plaintext payload before encryption may include:

- complete registry records
- secret bundles
- MCP settings without resolved environment overrides
- workspace registry
- app settings
- format/version metadata

Encryption rules:

- Prompt for passphrase twice.
- Passphrase never crosses TypeScript if the chosen Tauri command can obtain it through a secure native flow; if it must cross, keep it only in component memory and clear immediately.
- Do not store passphrase.
- Zeroize passphrase and plaintext buffers where supported.
- Use armored `.age` output.
- Write atomically.
- Include a checksum inside the encrypted payload.
- Restore always starts with preview.

### Import preview

Return:

```ts
interface StorageImportPreview {
  previewId: string;
  baseRegistryRevision: string;
  additions: StorageImportChange[];
  updates: StorageImportChange[];
  renames: StorageImportChange[];
  removals: StorageImportChange[];
  policyChanges: StorageImportChange[];
  exposureChanges: StorageImportChange[];
  missingSecretFields: MissingSecretField[];
  warnings: string[];
}
```

Rules:

- Preview ID expires in 10 minutes.
- Apply requires the same `baseRegistryRevision`.
- If registry changed, return `ERR_IMPORT_PREVIEW_STALE`.
- Default mode is merge.
- Default conflict handling is error.
- Replace mode requires typed confirmation.
- Before apply, create a registry/workspace/settings backup.
- Apply registry and secret changes transactionally.
- Never auto-enable MCP exposure from an imported shareable config.

### Raw JSON editor

- The editor must load public config only.
- It must not reveal keyring values.
- Secret fields are edited through explicit secret controls, not raw JSON.
- Saving raw JSON cannot clear or replace a credential.
- Consider renaming to `Advanced public configuration`.

### Required tests

- Shareable export has no secret value/ref.
- Recovery backup decrypts with correct passphrase and fails with wrong passphrase.
- Import preview accurately reports add/update/remove/exposure changes.
- Registry change invalidates preview.
- Failed apply restores original state.
- Imported shareable storage is not MCP-exposed.
- Replace mode requires explicit confirmation.
- Existing keyring secrets survive public-config edits.

### Acceptance criteria

- No UI path performs a secret-inclusive plaintext export.
- Import never silently replaces the complete registry.
- Recovery is possible from an encrypted backup.

---

## PR 05 — Version synchronization and bundled MCP sidecar

### Goal

Make the normal desktop installation self-contained and ensure desktop and MCP versions match.

### Primary files

```text
Cargo.toml
crates/core/Cargo.toml
crates/mcp/Cargo.toml
crates/mcp/src/main.rs
apps/desktop/src-tauri/Cargo.toml
apps/desktop/src-tauri/tauri.conf.json
apps/desktop/src-tauri/src/sidecar.rs       # new
apps/desktop/src-tauri/src/state.rs
apps/desktop/src-tauri/src/commands/mcp.rs
apps/desktop/src-tauri/binaries/.gitkeep    # new
apps/desktop/package.json
package.json
scripts/prepare-mcp-sidecar.mjs             # new
scripts/sync-release-version.mjs
scripts/check-release-consistency.mjs
scripts/smoke-mcp-sidecar.sh                 # new
.github/workflows/release.yml
docs/mcp-client-setup.md
```

### Workspace versioning

Add to root `Cargo.toml`:

```toml
[workspace.package]
version = "0.8.0"
edition = "2021"
rust-version = "1.85"
```

Use in all Rust packages:

```toml
version.workspace = true
edition.workspace = true
rust-version.workspace = true
```

Update release scripts so one tag version updates/checks:

- root workspace package version
- desktop package JSON
- Tauri config
- release notes
- changelog
- website metadata

The MCP server must report the same version as the desktop app.

### Sidecar build

Add to `tauri.conf.json`:

```json
{
  "bundle": {
    "externalBin": ["binaries/infimount_mcp"]
  }
}
```

Create `scripts/prepare-mcp-sidecar.mjs`.

Required behavior:

1. Accept optional `--target <triple>`.
2. Determine target with `rustc --print host-tuple` when omitted.
3. Build:

```bash
cargo build --release -p infimount_mcp --bin infimount_mcp --target <triple>
```

4. Copy:

```text
target/<triple>/release/infimount_mcp[.exe]
->
apps/desktop/src-tauri/binaries/infimount_mcp-<triple>[.exe]
```

5. Set executable permission on Unix.
6. Run `--version`.
7. Fail if the reported version differs from the desktop package version.
8. Print SHA-256.
9. Never commit generated binaries.

Add package scripts:

```json
{
  "scripts": {
    "build:mcp-sidecar": "node scripts/prepare-mcp-sidecar.mjs",
    "test:mcp-sidecar": "bash scripts/smoke-mcp-sidecar.sh"
  }
}
```

### Sidecar CLI

Replace manual argument scanning with a robust parser.

Commands/options:

```text
infimount_mcp --version
infimount_mcp serve --transport stdio
infimount_mcp serve --transport http --bind 127.0.0.1 --port 7331
infimount_mcp doctor --json
infimount_mcp print-config-dir
```

Preserve compatibility with current `--transport stdio|http` syntax for at least v0.8.

### Sidecar locator

Create `SidecarLocator`:

```rust
pub struct McpSidecarInfo {
    pub path: PathBuf,
    pub exists: bool,
    pub executable: bool,
    pub version: Option<String>,
    pub compatible: bool,
}
```

Discovery:

1. Resolve the Tauri resource directory.
2. Check known external-binary locations for the platform.
3. Check the directory containing the desktop executable.
4. Validate each candidate by running `--version` with a 3-second timeout.
5. Select only a same-version candidate.
6. Never fall back to bare `infimount_mcp` on `PATH` for generated default snippets.
7. An advanced UI may show a separately installed binary, but onboarding must use the bundled binary.

### Client snippets

Generate stdio JSON with the verified absolute path:

```json
{
  "mcpServers": {
    "infimount": {
      "command": "/absolute/path/to/infimount_mcp",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

Rules:

- Properly JSON-escape Windows backslashes and paths with spaces.
- Do not include credentials.
- HTTP snippet uses a placeholder/environment reference, not the bearer token.

### Release workflow

Before every `pnpm tauri build` job:

1. Build the target-specific sidecar.
2. Verify its version.
3. Verify it is included in the bundle.
4. Run the sidecar smoke test.

Add post-bundle checks:

- Linux: locate in AppImage/DEB/RPM bundle and run `--version`.
- macOS: mount DMG, locate sidecar in `Infimount.app`, run `--version`.
- Windows: inspect bundle/install output and run the `.exe --version`.
- Ensure SBOM/provenance includes the sidecar.

Optionally publish standalone headless sidecar binaries, but this is not a substitute for bundling.

### Required tests

- Path with spaces.
- Non-ASCII installation path.
- Windows `.exe` handling.
- Sidecar missing.
- Sidecar not executable.
- Version mismatch.
- Same-version success.
- Generated JSON parses.
- Packaged artifact contains executable.

### Acceptance criteria

- Fresh installer users can use stdio without source build.
- The sidecar and desktop always report the same version.
- Release gates fail when sidecar is missing.

---

## PR 06 — Rust workspace service and browser-storage migration

### Goal

Make Agent Workspaces durable, transactional, and first-class.

### Primary files

```text
crates/core/src/workspaces.rs               # new
crates/core/src/lib.rs
crates/mcp/src/registry.rs
apps/desktop/src-tauri/src/state.rs
apps/desktop/src-tauri/src/commands/workspaces.rs   # new
apps/desktop/src-tauri/src/main.rs
apps/desktop/src/lib/api.ts
apps/desktop/src/lib/agentWorkspaces.ts
apps/desktop/src/components/AgentWorkspacesDialog.tsx
apps/desktop/src/types/workspaces.ts         # new
docs/agent-workspaces.md                     # new
```

### Workspace registry

Path:

```text
~/.infimount/workspaces.json
```

Use:

- file lock
- atomic writes
- schema version
- Unix `0600`
- unique workspace ID/name validation
- storage existence validation

### Workspace creation command

```rust
create_agent_workspace(input) -> WorkspaceRecord
```

Input:

```rust
pub struct CreateWorkspaceInput {
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    pub access_profile: WorkspaceAccessProfile,
    pub adopt_existing: bool,
}
```

Algorithm:

1. Validate storage exists and is enabled.
2. Normalize root to backend form without leading/trailing slash.
3. Reject empty root, `/`, `.`, `..`, or any traversal.
4. Reject root overlapping an existing workspace root on the same storage:
   - equal
   - ancestor
   - descendant
5. Stat root.
6. If root exists and is non-empty, require `adopt_existing = true`.
7. Build template file plan.
8. For a new root, record every created path for rollback.
9. Create directories/files through OpenDAL.
10. Write `.infimount/workspace.json`.
11. Add a workspace-sourced policy rule:
    - ID `workspace:<workspace-id>`
    - prefix = normalized root
    - access from profile
12. Persist storage policy and workspace registry.
13. If any step fails:
    - restore previous storage policy
    - remove only files/directories created by this operation
    - never delete pre-existing adopted content
14. Emit local activity and audit metadata.

### Workspace deletion

Default action:

- remove workspace registry record
- remove its policy rule
- keep workspace files
- display that files remain

Separate destructive action:

```text
Delete workspace registration and files
```

It requires explicit confirmation and uses normal transfer/delete safety.

### Workspace updates

- Rename display name without changing path.
- Change access profile by updating only the associated rule.
- Do not permit arbitrary storage ID mutation.
- Moving root is a separate future operation; do not implement in v0.8.

### Browser-local migration

Current keys:

```text
infimount:agent-workspaces:v1
infimount:agent-workspace-checkpoints:v1
```

Migration:

1. Frontend reads old values once.
2. Calls `import_legacy_workspaces`.
3. Backend validates each storage/root/manifest.
4. Backend creates workspace records and rules safely.
5. Return per-workspace success/failure.
6. Remove a localStorage entry only after successful import.
7. Preserve failed entries and show recovery information.
8. Record migration completion in app settings.

### Checkpoints

Keep manifests in storage. Persist checkpoint index in the Rust workspace registry.

- Maximum 200 checkpoint metadata entries per workspace.
- Checkpoint restore requires write permission and confirmation when configured.
- Do not keep full checkpoint content in browser localStorage.

### Required tests

- New workspace creation.
- Adopt existing folder.
- Overlap rejection.
- Multiple workspaces on same storage.
- Transaction rollback.
- Delete registration without deleting files.
- Destructive delete confirmation.
- Legacy localStorage import.
- Corrupt manifest recovery.
- Workspace profile change updates only one rule.

### Acceptance criteria

- Reloading/reinstalling the webview does not lose workspace metadata.
- Workspace access maps to explicit policy rules.
- Multiple workspaces are independent.

---

## PR 07 — Workspace-first activation wizard and client integrations

### Goal

Replace informational onboarding with a verified install-to-first-read workflow.

### Primary files

```text
apps/desktop/src-tauri/src/app_settings.rs
apps/desktop/src-tauri/src/client_integrations.rs     # new
apps/desktop/src-tauri/src/mcp_probe.rs               # new
apps/desktop/src-tauri/src/commands/clients.rs        # new
apps/desktop/src-tauri/src/commands/mcp.rs
apps/desktop/src-tauri/src/commands/workspaces.rs
apps/desktop/src-tauri/src/main.rs
apps/desktop/src/types/storage.ts
apps/desktop/src/types/activation.ts                  # new
apps/desktop/src/lib/api.ts
apps/desktop/src/components/ActivationWizard.tsx      # new
apps/desktop/src/components/FirstRunOnboardingDialog.tsx
apps/desktop/src/pages/Index.tsx
docs/client-integrations/*
```

Delete or stop rendering the old completion dialog after the new wizard is wired.

### Wizard steps

#### Step 1 — Safety baseline

Display and apply:

- safe default tool set
- no admin tools
- read-only workspace default
- no whole-storage exposure
- local-only data statement

The user can continue with defaults without advanced settings.

#### Step 2 — Storage

Options:

- Create a local demo storage automatically.
- Use an existing storage.
- Add and validate a real storage.

Demo layout:

```text
<temp-or-config-demo-root>/
  workspace/
    README.md
    sample.txt
  outside/
    denied.txt
```

Do not expose the demo root as a whole.

#### Step 3 — Workspace

Create a read-only workspace at `workspace/`.

The policy must permit:

```text
workspace/**
```

and deny:

```text
outside/**
```

#### Step 4 — Sidecar

Run:

- locator
- version check
- `doctor --json`

Block progress on failure and show exact fix.

#### Step 5 — Client

Create a client-adapter interface:

```rust
pub trait McpClientAdapter {
    fn kind(&self) -> McpClientKind;
    fn detect(&self) -> ClientDetection;
    fn preview_install(&self, input: ClientInstallInput) -> Result<ClientInstallPreview>;
    fn apply_install(&self, preview_id: &str) -> Result<ClientInstallResult>;
}
```

Initial adapters:

1. Generic stdio JSON — always supported, copy only.
2. Claude Code — generate verified `claude mcp add` command; execution requires explicit confirmation.
3. Cursor — merge into selected project `.cursor/mcp.json` or global `~/.cursor/mcp.json`; create backup.
4. VS Code — prefer `code --add-mcp` when CLI is detected; otherwise produce `.vscode/mcp.json` preview.
5. OpenCode — generate current local MCP config snippet; automatic editing only for plain JSON files that parse without comments. Otherwise copy-only.
6. Claude Desktop — copy-only JSON unless config-path detection has platform tests.

Every write-capable adapter must:

- preview exact before/after
- create timestamped backup
- merge only the `infimount` entry
- preserve unrelated servers
- refuse malformed config rather than overwrite
- support rollback

#### Step 6 — End-to-end probe

Spawn the bundled sidecar with stdio and implement a minimal MCP client probe.

Required protocol sequence:

1. Start process.
2. Send initialize.
3. Receive initialize response.
4. Send initialized notification if required by protocol version.
5. Request `tools/list`.
6. Verify safe tool set and absence of admin tools.
7. Call `list_dir` on workspace path.
8. Call `read_file` on `sample.txt`.
9. Call `read_file` on `outside/denied.txt`.
10. Verify the final call returns `ERR_MCP_POLICY_DENIED`.
11. Stop process cleanly.
12. Kill on timeout.

Timeouts:

```text
process start: 5 seconds
initialize: 5 seconds
each request: 10 seconds
total probe: 30 seconds
```

Sanitize and truncate stderr to 8 KiB.

### Completion behavior

- `Finish setup` is enabled only after the probe passes.
- `Skip` remains available, but app displays a persistent incomplete-setup banner.
- Store timestamps for each verified event.
- The normal `Test connection` button reuses the same real probe; it must not only count tools/storages.

### Required tests

- Complete demo activation.
- Sidecar missing/version mismatch.
- MCP initialize timeout.
- Tool list contains admin tool -> fail.
- Allowed read success.
- Outside-scope read unexpectedly succeeds -> fail.
- Cursor/VS Code config merge preserves unrelated entries.
- Malformed client config is not overwritten.
- Rollback restores client config.
- Wizard resumes after app restart.

### Acceptance criteria

- A clean-machine user can finish with no external storage credentials.
- Completion proves both allowed and denied behavior.
- Median observed design-partner activation can be measured from local timestamps.

---

## PR 08 — Diagnostics and privacy-preserving product events

### Goal

Make failures diagnosable without compromising local-first privacy.

### Primary files

```text
crates/mcp/src/telemetry.rs
crates/mcp/src/audit.rs
crates/mcp/src/errors.rs
apps/desktop/src-tauri/src/diagnostics.rs        # new
apps/desktop/src-tauri/src/product_events.rs     # new
apps/desktop/src-tauri/src/commands/diagnostics.rs
apps/desktop/src-tauri/src/app_settings.rs
apps/desktop/src/components/DiagnosticsDialog.tsx
apps/desktop/src/components/PrivacySettings.tsx
apps/desktop/src/lib/api.ts
apps/desktop/src/types/diagnostics.ts
docs/privacy.md
docs/troubleshooting.md
```

### Diagnostics model

May include:

- app version
- sidecar version/path compatibility
- OS and architecture
- keyring status enum
- config file existence and permission status
- registry/workspace/settings schema versions
- number of storages
- coarse backend counts
- number of exposed storages
- enabled tool names
- HTTP bind category: loopback/non-loopback
- port availability
- last sanitized error codes
- recent audit decisions with names/paths removed unless the user explicitly selects local-only detailed export

Must not include:

- storage names
- file paths
- bucket/container names
- endpoints
- config JSON
- credentials
- OAuth values
- bearer tokens
- file contents
- prompts
- presigned URLs

### Product event model

Allowed events:

```text
app_launched
onboarding_started
onboarding_step_completed
storage_added
storage_validation_completed
workspace_created
sidecar_verified
client_config_previewed
client_config_applied
mcp_probe_completed
activation_completed
```

Allowed properties:

- event schema version
- app version
- OS/arch
- backend type
- workspace template
- access profile
- client kind
- success boolean
- failure stage
- sanitized error code
- duration bucket

Do not include arbitrary strings.

### Storage and transport

- Always write a bounded local JSONL event file.
- Maximum 5,000 events or 5 MiB; rotate oldest.
- Network export disabled by default.
- `TelemetryConsent::Granted` may enable an OTLP sink only when an endpoint is explicitly configured.
- Consent can be revoked.
- Revocation stops future network export; local deletion is a separate button.
- Replace current telemetry no-op methods with real structured local events and optional OTLP metrics.

### Diagnostics bundle

Write:

```text
diagnostics-<timestamp>/
  summary.json
  sanitized-errors.json
  redaction-manifest.json
  checksums.txt
```

Zip only after content validation.

### Required tests

- Seed a corpus of secrets/paths/names and assert none appear in default bundle.
- Event schema rejects unknown properties.
- Consent defaults unknown/off.
- No network client is constructed without consent and endpoint.
- Rotation works.
- Sidecar failure is diagnosable by stage/error code.

### Acceptance criteria

- Support can diagnose activation failures from a safe bundle.
- Product-learning events are structured, bounded, and opt-in for network export.

---

## PR 09 — Operator cache, paginated listing, and streaming I/O foundation

### Goal

Prevent large remote directories and files from causing excessive requests or memory usage.

### Primary files

```text
crates/core/src/runtime.rs                 # new
crates/core/src/registry.rs
crates/core/src/operations.rs
crates/core/src/models.rs
crates/mcp/src/opendal_adapter.rs
crates/mcp/src/tools_fs/common.rs
crates/mcp/src/tools_fs/list_dir.rs
crates/mcp/src/tools_fs/read_file.rs
apps/desktop/src-tauri/src/state.rs
apps/desktop/src-tauri/src/commands/storage.rs
apps/desktop/src-tauri/src/commands/transfers.rs
apps/desktop/src/lib/api.ts
apps/desktop/src/components/FileBrowser.tsx
apps/desktop/src/components/FilePreviewPanel.tsx
scripts/benchmark-large-storage.sh        # new
```

### Shared storage runtime

Create:

```rust
pub struct StorageRuntime {
    pub registry: StorageRegistry,
    pub secret_store: Arc<dyn SecretStore>,
    operator_cache: OperatorCache,
}
```

Cache key:

```text
storage_id + revision
```

Rules:

- Increment revision on any public config, secret, backend, root, or credential change.
- Invalidate cache on storage update/remove.
- Do not hash/log secret material.
- Desktop and MCP contexts use `StorageRuntime`.
- Do not construct an operator on every command.

### Listing API

Add:

```rust
pub struct ListEntriesPageRequest {
    pub storage_id: String,
    pub path: String,
    pub limit: u32,
    pub cursor: Option<String>,
    pub recursive: bool,
}

pub struct ListEntriesPage {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}
```

Rules:

- Non-recursive default limit: 200.
- Maximum limit: 1,000.
- Do not collect the full directory before slicing.
- Stop the lister once enough entries are available for the page.
- Use metadata returned by the lister when available.
- Enrich missing metadata with bounded concurrency, maximum 16.
- Avoid unconditional per-entry `stat`.
- Cursor includes:
  - version
  - storage revision
  - normalized path
  - recursive flag
  - deterministic offset or resume state
- Reject cursor when storage revision/query shape changes.
- Recursive listing:
  - default maximum 10,000 returned items
  - cancellable
  - return `truncated = true`
  - never allocate an unbounded tree

Update MCP `list_dir` to use the paged core API instead of collecting then paginating.

Update desktop file browser to use incremental pages.

### Range reads

Add:

```rust
read_file_range(storage_id, path, offset, max_bytes)
```

- Default preview max: 256 KiB.
- Absolute maximum through Tauri: 2 MiB.
- Return `total_size`, `offset`, `bytes`, `truncated`.
- Preview panel uses range reads.
- Full-file read remains only for bounded small operations and must enforce a maximum.

### Streaming transfers

Refactor upload/copy/move fallback:

- Stream through bounded buffers.
- Suggested buffer: 8 MiB.
- Emit progress per chunk.
- Never convert a multi-gigabyte file to `Vec<u8>` or JavaScript `number[]`.
- Preserve server-side copy when capability exists.
- Verify transferred byte count.
- Support cancellation between chunks.
- Keep conflict semantics unchanged.

### Benchmarks

Add reproducible simulator benchmarks:

```text
10,000-object non-recursive listing
100,000-object recursive capped search
1 GiB local-to-S3-simulator upload
1 GiB S3-simulator-to-local download
cross-storage copy
200 ms injected latency
cancel during planning
cancel during streaming
```

Record:

- time to first page
- total requests where measurable
- peak RSS
- bytes transferred
- cancellation latency

### Required tests

- First page does not enumerate full directory.
- Metadata enrichment concurrency cap.
- Cursor invalidation.
- Recursive cap/truncation.
- Range read boundaries.
- Transfer memory remains bounded in integration test.
- Cache hit/miss/invalidation.
- Secret update invalidates operator.

### Acceptance criteria

- 10,000-object directory displays its first page without loading all entries.
- Large transfers do not materialize complete files in Rust or TypeScript memory.
- Desktop and MCP share cached operator construction.

---

## PR 10 — Tauri hardening, signing, and release publication

### Goal

Make stable builds trustworthy and eliminate insecure stable-release fallbacks.

### Primary files

```text
apps/desktop/src-tauri/tauri.conf.json
apps/desktop/src-tauri/capabilities/main.json
apps/desktop/src-tauri/src/main.rs
.github/workflows/release.yml
scripts/check-release-assets.sh
scripts/check-release-consistency.mjs
scripts/release-test-gate.sh
docs/releasing.md
docs/security.md
```

### Tauri configuration

- Set `withGlobalTauri` to false unless a documented runtime dependency proves it is required.
- Add a restrictive CSP.
- Required sources must be justified by actual app behavior:
  - self
  - Tauri IPC
  - blob/data images for previews where needed
- Deny:
  - arbitrary remote scripts
  - arbitrary remote frames
  - object embedding
  - unrestricted connect sources
- Keep sidecar execution in Rust; do not grant general shell execution to the webview.
- Review every capability permission and remove unused entries.
- Add tests that core UI and previews work under CSP.

### Stable signing policy

A stable tag matches:

```regex
^v[0-9]+\.[0-9]+\.[0-9]+$
```

For a stable tag:

- macOS signing certificate, identity, Apple credentials, team ID, and notarization must exist.
- Windows signing certificate and password must exist.
- Tauri updater signing key must exist.
- Missing signing material fails the workflow.
- No unsigned stable fallback.

For prerelease tags:

- Unsigned output may be permitted only with a prominent `UNSIGNED` marker and prerelease status.
- It must not update the stable `latest` channel.

### Release publication

Preferred workflow:

1. Build and sign.
2. Create draft release.
3. Download public-equivalent assets.
4. Run install-link, checksum, signature, updater, sidecar, and launch validation.
5. Publish release automatically only if all post-release checks pass.
6. Update Homebrew/other channels after publication.
7. Verify `releases/latest` resolves to the new stable release.

Do not leave the stable release as a draft after a successful pipeline.

### Artifact checks

Verify:

- installer signatures
- notarization result
- updater signatures
- sidecar presence/version
- checksums
- SBOM
- provenance
- install scripts
- release version consistency
- no secret test fixtures in artifacts

### Acceptance criteria

- Stable workflow cannot produce unsigned macOS/Windows artifacts.
- Stable release publishes after all gates.
- README, website, updater, Homebrew, and GitHub latest version agree.

---

## PR 11 — Documentation, migration guide, and v0.8 release

### Goal

Align the public product with the implemented agent-storage permission layer.

### Primary files

```text
README.md
CHANGELOG.md
docs/index.html
docs/llms.txt
docs/security.md
docs/mcp-client-setup.md
docs/agent-integrations.md
docs/agent-workspaces.md
docs/recovery.md
docs/troubleshooting.md
docs/releasing.md
docs/migration-v0.8.md
docs/release-notes-0.8.0.md
```

### README structure

1. Headline: `Safe storage access for AI agents`.
2. One-paragraph value proposition.
3. 60-second workspace workflow.
4. Installation.
5. Supported clients.
6. Security defaults.
7. Storage providers.
8. Desktop file-manager capabilities.
9. Architecture.
10. Contributing.

Do not lead with the provider list.

### Required documentation

- v0.7 -> v0.8 credential migration.
- Keyring availability and Linux behavior.
- Admin MCP tool removal.
- Policy v2 semantics.
- Workspace access profiles.
- Recovery backup/restore.
- Bundled sidecar locations.
- Client setup and rollback.
- Diagnostics bundle contents.
- Telemetry consent and event schema.
- Signing verification.
- Known limitations.

### Release sequence

1. Release `v0.8.0-rc.1`.
2. Run clean-install tests on all three OS families.
3. Complete at least 10 observed design-partner activations.
4. Fix blockers only; no new features.
5. Release `v0.8.0`.
6. Verify:
   - latest links
   - update channel
   - Homebrew
   - sidecar version
   - signatures
   - docs
7. Publish problem-focused launch material.

### Acceptance criteria

- Public claims match shipped behavior.
- No documentation instructs users to build `infimount_mcp` separately for normal desktop use.
- No documentation suggests plaintext credential exports.

---

# 7. New error codes

Add to `McpErrorCode` or a shared app error enum as appropriate:

```text
ERR_SECURITY_BASELINE_REQUIRED
ERR_ADMIN_TOOL_UNAVAILABLE
ERR_SECRET_STORE_UNAVAILABLE
ERR_SECRET_STORE_LOCKED
ERR_SECRET_NOT_FOUND
ERR_SECRET_MIGRATION_FAILED
ERR_SECRET_UPDATE_FAILED
ERR_OAUTH_SESSION_NOT_FOUND
ERR_OAUTH_SESSION_EXPIRED
ERR_OAUTH_SESSION_ALREADY_USED
ERR_POLICY_VERSION_UNSUPPORTED
ERR_POLICY_RULE_CONFLICT
ERR_WORKSPACE_NOT_FOUND
ERR_WORKSPACE_PATH_INVALID
ERR_WORKSPACE_PATH_OVERLAP
ERR_WORKSPACE_TRANSACTION_FAILED
ERR_SIDECAR_NOT_FOUND
ERR_SIDECAR_NOT_EXECUTABLE
ERR_SIDECAR_VERSION_MISMATCH
ERR_CLIENT_NOT_DETECTED
ERR_CLIENT_CONFIG_INVALID
ERR_CLIENT_CONFIG_CONFLICT
ERR_CLIENT_CONFIG_APPLY_FAILED
ERR_MCP_PROBE_START_FAILED
ERR_MCP_PROBE_TIMEOUT
ERR_MCP_PROBE_PROTOCOL_FAILED
ERR_MCP_PROBE_SCOPE_FAILED
ERR_IMPORT_PREVIEW_STALE
ERR_BACKUP_DECRYPT_FAILED
ERR_BACKUP_SCHEMA_UNSUPPORTED
```

Error details must contain identifiers and error codes, not sensitive raw provider text.

---

# 8. Complete validation matrix

## 8.1 Required commands on every final PR branch

```bash
pnpm install --frozen-lockfile

pnpm --dir apps/desktop lint
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop test:integration
pnpm --dir apps/desktop test:ui
pnpm --dir apps/desktop build

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- \
  -D warnings \
  -A clippy::result_large_err \
  -A clippy::needless_borrows_for_generic_args
cargo test --workspace

./scripts/smoke-desktop.sh
./scripts/storage-simulator-gate.sh
./scripts/smoke-mcp-sidecar.sh
./scripts/smoke-activation.sh
./scripts/smoke-secret-migration.sh
./scripts/smoke-backup-restore.sh
```

## 8.2 Upgrade fixtures

Create committed secret-free test fixtures that use obvious fake values:

```text
tests/fixtures/v0.7/storages-plaintext.json
tests/fixtures/v0.7/mcp-settings-all-tools.json
tests/fixtures/v0.7/app-settings.json
tests/fixtures/v0.7/workspaces-localstorage.json
tests/fixtures/v0.8/shareable-config.json
tests/fixtures/v0.8/recovery-payload.json
```

Seed credentials such as:

```text
TEST_SECRET_ACCESS_KEY_DO_NOT_SHIP
TEST_OAUTH_REFRESH_TOKEN_DO_NOT_SHIP
TEST_HTTP_BEARER_TOKEN_DO_NOT_SHIP
```

The artifact scan must fail if these values appear in build outputs.

## 8.3 Security scenarios

- Unexposed storage is invisible.
- Exposed storage with no grants is inaccessible.
- Read-only workspace read succeeds.
- Read-only workspace write fails.
- Destructive tool disabled means undiscoverable and uncallable.
- Approval cannot be replayed for another path.
- Session cannot elevate policy.
- Path traversal and encoding bypasses fail.
- Removed admin tool call fails.
- Shareable export contains no credentials.
- Diagnostics contain no seeded names/paths/secrets.
- OAuth token does not cross webview.
- Non-loopback HTTP cannot start without resolved auth.
- Stable release fails without signing.

## 8.4 Platform scenarios

### Linux

- DEB, RPM, AppImage.
- Keyring available.
- Keyring unavailable produces clear error.
- AppImage sidecar path.
- Home directory with spaces/non-ASCII.
- Loopback HTTP and stdio.

### macOS

- Signed/notarized DMG.
- Apple Silicon and Intel where supported.
- Sidecar inside app bundle.
- Keychain access.
- Generated absolute path.
- Upgrade through updater.

### Windows

- Signed MSI and NSIS EXE.
- Credential Manager access.
- Sidecar `.exe`.
- Paths with spaces and backslashes.
- SmartScreen/signature verification.
- stdio client process launch.

---

# 9. Release-blocking definition of done

v0.8 may ship only when every statement is true:

- [ ] Public MCP discovery contains no storage administration tools.
- [ ] Fresh settings use the exact safe tool set.
- [ ] Legacy all-tool settings migrate safely with backup.
- [ ] Policy v2 migration passes all fixtures.
- [ ] Current workspace policy bug is repaired.
- [ ] Multiple workspaces on one storage work.
- [ ] No plaintext credential remains in config after migration.
- [ ] OAuth tokens do not enter TypeScript responses.
- [ ] MCP bearer token is absent from settings JSON.
- [ ] Shareable export is secret-free.
- [ ] Encrypted recovery backup/restore passes.
- [ ] Import requires preview and revision match.
- [ ] Desktop bundles the same-version sidecar.
- [ ] Generated stdio config uses a verified absolute path.
- [ ] Workspace records survive reload and migration from localStorage.
- [ ] Real MCP probe proves allowed read and denied outside read.
- [ ] Diagnostics and events pass redaction tests.
- [ ] First large-directory page does not load the full directory.
- [ ] Large transfers are streamed with bounded memory.
- [ ] Tauri CSP and capability hardening tests pass.
- [ ] Stable release cannot be unsigned.
- [ ] Stable GitHub release is published and matches all public channels.
- [ ] Documentation matches shipped behavior.
- [ ] No new storage backend was added.

---

# 10. Recommended OpenCode execution prompt

Use this prompt for each PR, replacing the PR number and scope:

```text
You are implementing Infimount v0.8 from
docs/roadmaps/v0.8-trust-activation.md.

Implement only PR <NUMBER>: <TITLE>.

Rules:
1. Read the full PR section, all referenced ADRs, and every affected file before editing.
2. Do not implement later PRs.
3. Preserve existing behavior unless the PR explicitly changes it.
4. Add or update tests for every acceptance criterion.
5. Never log or persist credentials, tokens, file contents, or raw storage config.
6. Run all validation commands listed for this PR.
7. Report:
   - files changed
   - migrations added
   - tests added
   - commands run and results
   - remaining risks
8. Do not claim completion if any acceptance criterion is unverified.
```

For the final integration branch:

```text
Audit the complete v0.8 implementation against every checkbox in
section 9 of docs/roadmaps/v0.8-trust-activation.md.

Do not add features. Fix only deviations, missing tests, migration issues,
security regressions, packaging failures, and documentation inconsistencies.
Produce a matrix mapping each checkbox to code and test evidence.
```

---

# 11. Dependency graph

```text
PR 00
  └── PR 01
        └── PR 02
              └── PR 03
                    ├── PR 04
                    └── PR 05
                          └── PR 06
                                └── PR 07
                                      ├── PR 08
                                      └── PR 09
                                            └── PR 10
                                                  └── PR 11
```

Do not begin PR 06 before policy v2, secret storage, and sidecar contracts are stable.
Do not begin public launch work before the activation probe, signing, and clean-install gates pass.

---

# 12. Final product acceptance scenario

The canonical final E2E scenario is:

1. Install signed Infimount on a clean machine.
2. Launch the app.
3. Accept the safe baseline.
4. Create the demo storage.
5. Create the default read-only workspace.
6. Verify the bundled sidecar.
7. Generate or apply an MCP configuration.
8. Run the internal MCP probe.
9. Confirm:
   - workspace list succeeds
   - sample file read succeeds
   - outside file read is denied
   - no admin tool is listed
10. Open the audit viewer and see:
    - allowed workspace read
    - denied outside-scope read
    - matched workspace policy rule
11. Complete onboarding.
12. Close and reopen the app.
13. Re-run the probe successfully.
14. Upgrade to the same version through a test updater channel without losing:
    - storage records
    - keyring credentials
    - workspace records
    - policies
    - client configuration
    - audit integrity

This scenario must pass on Linux, macOS, and Windows release artifacts.
