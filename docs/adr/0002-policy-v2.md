# ADR 0002: Policy Schema v2

**Status:** Accepted  
**Date:** 2026-07-18  
**Driver:** v0.8 Trust & Activation  

## Context

The current MCP storage policy uses a legacy allow-list-as-restriction-only
model (`allowed_paths`) with ambiguous default-access semantics. Agent
Workspaces created through the current code have a bug where the policy
grants no effective access, and there is no mechanism for:

- Explicit per-prefix grants with different access modes.
- Multiple independent workspace grants on the same storage.
- A global deny that overrides all grants.
- Tracking which rule (or workspace) produced a given access decision.

## Decision

Replace the single `allowed_paths` list with an explicit rules-based policy
schema (version 2) that uses a longest-prefix-match evaluation algorithm.

## Affected files

- `crates/mcp/src/policy.rs`
- `crates/mcp/src/registry.rs`
- `crates/mcp/src/tools_fs/common.rs`
- `crates/mcp/src/server.rs`
- `crates/mcp/src/session.rs`
- `crates/mcp/src/audit.rs`
- `apps/desktop/src/types/storage.ts`
- `apps/desktop/src/pages/Index.tsx`
- `apps/desktop/src/components/mcp/McpPolicySection.tsx`
- `apps/desktop/src/lib/agentWorkspaces.ts`
- `docs/security.md`
- `docs/mcp-client-setup.md`

## Contract

### Policy version

```rust
pub const MCP_POLICY_VERSION: u32 = 2;
```

### Rule source

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpRuleSource {
    Manual,
    Workspace { workspace_id: String },
}
```

### Path rule

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPathRule {
    pub id: String,
    pub prefix: String,
    pub access: McpAccessMode,
    pub source: McpRuleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_rules: Option<McpConfirmationRules>,
}
```

### Storage policy

```rust
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

### New default

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

### Evaluation algorithm

`evaluate_storage_policy()` executes in this exact order:

1. Reject when `storage.mcp_exposed == false`.
2. Normalize the requested backend path.
3. Reject if any `denied_paths` prefix matches.
4. Find all matching `rules`; choose the rule with the longest normalized
   prefix.
5. Reject duplicate rules with the same normalized prefix during
   normalization. Do not resolve ambiguity at request time.
6. Effective access is the matched rule's access; otherwise it is
   `default_access`.
7. If `storage.read_only` is true, any write-like operation is rejected
   regardless of the rule.
8. If effective access is `none`, reject.
9. If effective access is `read_only` and the operation is write-like,
   reject.
10. Use the matched rule's confirmation rules when present; otherwise use
    policy-level confirmation rules.
11. Return the decision plus `matched_rule_id` and optional `workspace_id`
    for audit.

### Return model

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
- Rule access mapping:
  - legacy `read_write` → `read_write`
  - legacy `read_only` → `read_only`
  - legacy `none` → `read_only` (safety repair for existing workspace bug)
- Set `default_access = none`.
- Preserve `denied_paths` and confirmation rules.
- Set `version = 2`.
- Save the migrated registry atomically.
- Create a pre-migration backup first.

When legacy `allowed_paths` is empty, preserve the legacy default access.

### Storage record changes

```rust
pub const STORAGE_RECORD_SCHEMA_VERSION: u32 = 2;

pub struct StorageRecord {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub backend: String,
    pub config: serde_json::Value,
    pub secret_ref: Option<String>,
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

`StorageRecord::new()` defaults:
- `mcp_exposed = false`
- policy v2 default (no access)

### Workspace policy rules

- Never set `default_access = none` while relying on legacy `allowed_paths`.
- Add or update one `McpPathRule` with source `workspace`.
- Default access profile is read-only.
- Preserve other rules.
- Workspace root may not be empty or `/`.

### Session creation validation

`create_session()` enforces these invariants before issuing a session:

- The requested storage must be `enabled == true` and `mcp_exposed == true`.
- The requested prefix must be inside an effective policy grant after
  evaluation (i.e. `evaluate_storage_policy` for the prefix must return
  a non-`none` decision).
- A session requested with write-like access must not be issued when the
  effective access for the prefix is `read_only` — a writable session
  cannot elevate a read-only grant.
- A session created without requesting any prefix (empty prefix list)
  is still bounded by the storage policy: every operation through the
  session is subject to `evaluate_storage_policy` at the actual path.

### Presets

- **Read-only research:** safe tool set; convert existing non-deny grants
  to read-only.
- **Workspace agent:** enable non-destructive write tools only; preserve
  each workspace's selected access profile.
- **Manual approval:** enable broad data-plane tools; preserve path grants;
  confirmation rules remain on.
- **Lock down:** empty tool set and set whole-storage default to none;
  do not delete workspace rules, mark them inactive through disabled tools.

## Consequences

- Agent Workspaces are actually accessible under their intended roots.
- Multiple workspace grants coexist without conflict.
- No path grant can silently expand access beyond its prefix.
- Every allowed/denied decision can identify the matched rule in audit.
- Legacy policies are migrated with backup and a safety repair for the
  workspace bug.
