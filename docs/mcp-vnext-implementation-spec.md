# Infimount vNext MCP Implementation Spec

Status: Planning Approved (No code changes in this document)  
Owner: Rajan  
Scope: Versioning parity + production MCP hardening for next release

## 1. Purpose

This document is the execution spec for coding agents. It translates the vNext PRD into:

1. Concrete gap analysis against current implementation.
2. Feasible scope for this release.
3. Low-level requirements and file-level change plan.
4. Strict acceptance criteria and test gates.

This is an implementation contract. If any item below conflicts with ad-hoc agent choices, this document wins.

## 2. Current Baseline (Grounded in repository)

### 2.1 Already implemented

1. MCP server with `stdio` and Streamable HTTP transport exists.
2. HTTP bind/port supports port `0` and returns actual runtime endpoint.
3. Tool-level enable/disable exists via MCP settings (`enabled_tools`).
4. Tool envelope and strict schemas are already in place.
5. Registry + MCP settings use atomic writes and lock timeout (`2s`).
6. Desktop MCP settings modal exists and controls runtime start/stop.
7. `validate_storage` already has a hard timeout (`60s`).

### 2.2 Missing vs PRD

1. Versioning tools are missing:
   - `list_versions`
   - `read_file_version`
   - `delete_version`
2. Versioning-specific error codes are missing:
   - `ERR_VERSIONS_NOT_SUPPORTED`
   - `ERR_VERSIONS_NOT_ENABLED`
3. Session policy/scoping tools are missing:
   - `session_create`
   - `session_end`
4. HTTP bearer auth enforcement is missing.
5. OTEL traces/metrics export is missing.
6. File detail UI version browsing is missing.
7. Optional web `/admin` UI is missing.

## 3. Feasibility and Scope Decisions

## 3.1 Feasible now

1. Versioning via OpenDAL capability and version options.
2. HTTP bearer token enforcement for MCP endpoint.
3. Session scoping in-process (memory-backed).
4. OTEL baseline instrumentation (tool-level + backend operation spans/metrics).
5. Desktop versions panel with capability gating.

## 3.2 Feasible with caveats

1. `ERR_VERSIONS_NOT_ENABLED` is backend-specific:
   - Deterministic detection for all backends is not guaranteed through generic OpenDAL APIs alone.
   - Implement deterministic mapping where backend errors are explicit.
   - For ambiguous cases, return `ERR_INTERNAL` with sanitized `details.backend_error`.

## 3.3 Deferred from this release

1. `restore_version` (skip unless a generic and deterministic cross-backend contract is defined).
2. Optional `/admin` web UI (keep as follow-up milestone).
3. Serve adapters (WebDAV/SFTP gateway).

## 4. Architecture Constraints

1. Storage source of truth remains:
   - `~/.infimount/storages.json`
2. MCP settings source of truth remains:
   - `~/.infimount/mcp_settings.json`
3. No external DB introduced.
4. Keep envelope unchanged:
   - Success: `{ "ok": true, "data": ... }`
   - Error: `{ "ok": false, "error": { code, message, details } }`
5. Never log secrets or raw storage config.

## 5. Contract Additions

## 5.1 New MCP tools (exact names)

1. `list_versions`
2. `read_file_version`
3. `delete_version`
4. `session_create`
5. `session_end`

## 5.2 Existing FS tools schema extension

Add optional `session_id` to filesystem-affecting tools:

1. `list_dir`
2. `stat_path`
3. `read_file`
4. `write_file`
5. `mkdir`
6. `move_path`
7. `copy_path`
8. `delete_path`
9. `search_paths`
10. `generate_download_link`
11. `read_file_version`
12. `list_versions`
13. `delete_version`

Rules:

1. `session_id` omitted => current behavior (no scoped session).
2. `session_id` present => enforce allowlists/prefix/read-only overrides.

## 5.3 New error codes

Add to `crates/mcp/src/errors.rs`:

1. `ERR_VERSIONS_NOT_SUPPORTED`
2. `ERR_VERSIONS_NOT_ENABLED`
3. `ERR_UNAUTHORIZED`
4. `ERR_SESSION_NOT_FOUND`
5. `ERR_SESSION_FORBIDDEN`

## 6. Detailed Phase Plan

## 6.1 Phase 1: Versioning parity (must ship)

Files:

1. `crates/mcp/src/tools_fs/list_versions.rs` (new)
2. `crates/mcp/src/tools_fs/read_file_version.rs` (new)
3. `crates/mcp/src/tools_fs/delete_version.rs` (new)
4. `crates/mcp/src/tools_fs/mod.rs`
5. `crates/mcp/src/schemas.rs`
6. `crates/mcp/src/server.rs`
7. `crates/mcp/src/errors.rs`
8. `crates/mcp/src/tools_fs/tests.rs`

Requirements:

1. `list_versions`
   - Input: `{ path, limit?, cursor?, session_id? }`
   - Output: `{ path, versions: [...], next_cursor? }`
   - Capability gate first. If unsupported -> `ERR_VERSIONS_NOT_SUPPORTED`.
2. `read_file_version`
   - Input mirrors `read_file` plus required `version`.
   - Same max bytes constraints as `read_file`.
3. `delete_version`
   - Input: `{ path, version, session_id? }`
   - Output: `{ path, version, deleted: true }`
4. Cursor
   - Use versioned base64url JSON cursor:
   - `{ "v": 1, "offset": <number> }`
5. Determinism
   - If backend returns unsorted versions, sort deterministically by `(modified_at desc, version asc)` as fallback.
6. Error mapping
   - Unsupported capability -> `ERR_VERSIONS_NOT_SUPPORTED`.
   - Explicit backend disabled signals -> `ERR_VERSIONS_NOT_ENABLED`.
   - Unknown backend failures -> mapped + sanitized backend detail.

Acceptance:

1. Versioning tools appear in `list_tools`.
2. Unsupported backend returns `ERR_VERSIONS_NOT_SUPPORTED`.
3. Supported backend basic version read/delete paths pass integration tests.

## 6.2 Phase 2: Capability parity guardrails

Files:

1. `crates/mcp/src/tools_storage/validate_storage.rs`
2. Optional: `crates/mcp/src/tools_storage/get_capabilities.rs` (new, only if needed)
3. `crates/mcp/src/schemas.rs`
4. `crates/mcp/src/server.rs`
5. `crates/mcp/src/tools_storage/tests.rs`

Requirements:

1. Extend capability summary fields with version-related booleans:
   - `list_with_versions`
   - `read_with_version`
   - `delete_with_version`
2. Keep old capability fields backward-compatible.
3. Do not infer by backend name. Use OpenDAL capability object.

Acceptance:

1. `validate_storage` returns versioning flags.
2. UI and MCP can gate version features without backend hardcoding.

## 6.3 Phase 3: HTTP auth hardening

Files:

1. `crates/mcp/src/main.rs`
2. `crates/mcp/src/runtime.rs`
3. `crates/mcp/src/errors.rs`
4. `crates/mcp/src/settings.rs` (only if adding explicit insecure toggle)
5. `apps/desktop/src-tauri/src/state.rs`
6. `apps/desktop/src/components/McpSettingsDialog.tsx` (status text)
7. Tests in MCP runtime/server layer

Requirements:

1. Env var: `INFIMOUNT_AUTH_TOKEN`.
2. HTTP start rules:
   - If HTTP requested and no token: fail startup unless `--allow-insecure` is explicitly passed.
3. Desktop defaults:
   - Bind default remains `127.0.0.1`.
4. Auth enforcement:
   - Require `Authorization: Bearer <token>`.
   - Missing/invalid token -> `ERR_UNAUTHORIZED`.
5. Security UX:
   - If bind is non-loopback (`0.0.0.0` or LAN IP), UI warns and requires explicit confirmation.

Acceptance:

1. HTTP server cannot run in secure mode without token.
2. Unauthorized requests fail deterministically.
3. Authorized requests continue to operate normally.

## 6.4 Phase 4: Session scoping and policy

Files:

1. `crates/mcp/src/session.rs` (new)
2. `crates/mcp/src/server.rs`
3. `crates/mcp/src/schemas.rs`
4. `crates/mcp/src/tools_fs/common.rs`
5. `crates/mcp/src/tools_fs/*.rs` (session enforcement)
6. `crates/mcp/src/errors.rs`
7. Tests in `crates/mcp/src/tools_fs/tests.rs` + new session tests

Requirements:

1. Tool: `session_create`
   - Input: `{ allowed_storages: [string], allowed_prefixes?: [string], read_only?: bool, ttl_seconds?: int }`
   - Output: `{ session_id }`
2. Tool: `session_end`
   - Input: `{ session_id }`
   - Output: `{ session_id, ended: true }`
3. Session validation on all FS/version tools:
   - Storage allowlist
   - Prefix allowlist
   - Read-only override
4. TTL enforcement:
   - Expired session behaves as `ERR_SESSION_NOT_FOUND`.
5. In-memory only for vNext (no persistence across restart).

Acceptance:

1. Access outside allowlist returns `ERR_SESSION_FORBIDDEN`.
2. Read-only session blocks mutating operations.
3. Expired/nonexistent session returns `ERR_SESSION_NOT_FOUND`.

## 6.5 Phase 5: OTEL baseline instrumentation

Files:

1. `crates/mcp/Cargo.toml`
2. `crates/mcp/src/main.rs`
3. `crates/mcp/src/server.rs`
4. `crates/mcp/src/tools_fs/common.rs` and key tool paths
5. Add dedicated telemetry module if needed: `crates/mcp/src/telemetry.rs` (new)

Requirements:

1. Support standard env vars:
   - `OTEL_EXPORTER_OTLP_ENDPOINT`
   - `OTEL_SERVICE_NAME`
   - `OTEL_RESOURCE_ATTRIBUTES`
2. Traces:
   - Span per tool invocation.
   - Child spans for backend operations where practical.
3. Metrics:
   - Counter: tool calls by tool name.
   - Counter: errors by error code.
   - Histogram: latency.
4. Logs:
   - Include trace context IDs when available.
5. If OTEL env vars are absent, run without exporter errors.

Acceptance:

1. Setting OTEL endpoint emits traces/metrics to collector.
2. No functional regressions when OTEL is disabled.

## 6.6 Phase 6: Desktop versions UI (must for G1)

Files:

1. `apps/desktop/src/components/FilePreviewPanel.tsx`
2. `apps/desktop/src/components/FilePreviewDialog.tsx` (if modal version is used)
3. `apps/desktop/src/lib/api.ts`
4. `apps/desktop/src-tauri/src/commands.rs`
5. `apps/desktop/src-tauri/src/state.rs` (wrappers to MCP tools)
6. Integration/UI tests in `apps/desktop/src/integration/` and `apps/desktop/playwright/`

Requirements:

1. Add Versions tab in file detail/preview for files only.
2. Show tab only when capability indicates version support.
3. Provide actions:
   - View/read selected version.
   - Download selected version.
   - Delete selected version (when allowed).
4. UI must surface explicit versioning errors:
   - not supported
   - not enabled
5. Keep styling consistent with current Infimount theme.

Acceptance:

1. Versions tab appears for supported storage/file.
2. Version operations work end-to-end via MCP.
3. Unsupported storages show clear gated UX.

## 7. Optional Admin UI (deferred)

Target is postponed unless time remains. If started:

1. Serve `/admin` only when `INFIMOUNT_ADMIN_UI=true`.
2. Reuse same bearer token auth.
3. Secret-safe policy:
   - Never reveal existing secrets.
   - Allow only replacement.

## 8. Schema Requirements (strict)

For all new/changed schemas:

1. `type: "object"`
2. `additionalProperties: false`
3. Exact required fields only
4. Defaults and min/max constraints where relevant

Versioning schema minimums:

1. `limit` defaults to `200`, min `1`, max `1000`.
2. `offset_bytes` default `0`, min `0`.
3. `max_bytes` default `262144`, min `1`, max `2097152`.

## 9. Security Requirements

1. Do not log raw request JSON bodies.
2. Do not log storage config values.
3. Do not log auth token or session internals.
4. Keep error details sanitized; backend error text allowed only under `details.backend_error`.
5. Resources must continue to route through tools, not direct OpenDAL bypass.

## 10. Test Plan (must pass)

## 10.1 Unit tests

1. Error mapping for new codes.
2. Cursor encode/decode and invalid cursor behavior.
3. Session TTL and policy enforcement.
4. Capability gating decisions.

## 10.2 Integration tests

1. Existing MCP tool regressions (all current tools).
2. Versioning:
   - supported path
   - unsupported path
   - disabled-if-detectable path
3. HTTP auth:
   - no token
   - bad token
   - valid token
4. Session scoped access and denial.

## 10.3 UI tests

1. Versions tab visibility and states.
2. Version action flows.
3. MCP settings warnings for non-loopback bind.

## 10.4 Workflow gates

All required workflows must pass before merge:

1. Repo Lint
2. CI
3. Integration Tests
4. Coverage

## 11. PR-by-PR Execution Checklist (strict)

This section is the task board. Agents must execute PRs in this exact order.

Global rules for every PR:

1. Keep PR scope limited to the phase.
2. Include tests in the same PR.
3. Do not modify release workflows unless explicitly required by the phase.
4. Preserve existing non-loopback bind warning behavior in MCP settings UI.
5. Add changelog/doc note when user-visible behavior changes.

### 11.1 PR-1: Versioning tools and schema/error contract

Title:

1. `feat(mcp): add versioning tools and explicit versioning error codes`

In scope:

1. Add `list_versions`, `read_file_version`, `delete_version`.
2. Add error codes:
   - `ERR_VERSIONS_NOT_SUPPORTED`
   - `ERR_VERSIONS_NOT_ENABLED`
3. Add strict schemas for new tools.
4. Register tools in MCP server and tool definitions.
5. Add deterministic cursor format and parsing for version listing.

Out of scope:

1. Session policy.
2. HTTP auth.
3. OTEL.
4. Desktop versions UI.

Required files:

1. `crates/mcp/src/tools_fs/list_versions.rs` (new)
2. `crates/mcp/src/tools_fs/read_file_version.rs` (new)
3. `crates/mcp/src/tools_fs/delete_version.rs` (new)
4. `crates/mcp/src/tools_fs/mod.rs`
5. `crates/mcp/src/schemas.rs`
6. `crates/mcp/src/server.rs`
7. `crates/mcp/src/errors.rs`
8. `crates/mcp/src/tools_fs/tests.rs`

Acceptance commands:

```bash
cargo fmt --all
cargo clippy -p infimount_mcp --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
cargo test -p infimount_mcp
```

Merge criteria:

1. New tools show in MCP `list_tools`.
2. Unsupported backend path returns `ERR_VERSIONS_NOT_SUPPORTED`.

### 11.2 PR-2: Capability parity extension

Title:

1. `feat(mcp): extend validate_storage capability matrix with versioning flags`

In scope:

1. Extend `validate_storage` output with versioning capability booleans.
2. Keep existing capability fields backward compatible.
3. Add tests for new capability fields.

Out of scope:

1. New standalone `get_capabilities` tool (unless approved).

Required files:

1. `crates/mcp/src/tools_storage/validate_storage.rs`
2. `crates/mcp/src/tools_storage/tests.rs`
3. `crates/mcp/src/server.rs` (only if metadata descriptions need updates)

Acceptance commands:

```bash
cargo fmt --all
cargo clippy -p infimount_mcp --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
cargo test -p infimount_mcp
```

Merge criteria:

1. Capability matrix includes versioning support fields.

### 11.3 PR-3: HTTP auth hardening

Title:

1. `feat(mcp): enforce bearer auth for HTTP transport`

In scope:

1. Add `INFIMOUNT_AUTH_TOKEN` support.
2. Add `--allow-insecure` dev override behavior.
3. Reject HTTP startup without token unless insecure override is explicit.
4. Enforce bearer token on HTTP requests.
5. Return `ERR_UNAUTHORIZED` for missing/invalid auth.

Out of scope:

1. Session policy.
2. OTEL.
3. Versions UI.

Required files:

1. `crates/mcp/src/main.rs`
2. `crates/mcp/src/runtime.rs`
3. `crates/mcp/src/errors.rs`
4. Tests in MCP runtime/server layer

Acceptance commands:

```bash
cargo fmt --all
cargo clippy -p infimount_mcp --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
cargo test -p infimount_mcp
```

Merge criteria:

1. Unauthorized HTTP requests fail deterministically.
2. Authenticated requests continue to work.

### 11.4 PR-4: Session scoping and enforcement

Title:

1. `feat(mcp): add session_create/session_end and scoped filesystem access`

In scope:

1. Add `session_create` and `session_end`.
2. Add error codes:
   - `ERR_SESSION_NOT_FOUND`
   - `ERR_SESSION_FORBIDDEN`
3. Add optional `session_id` to all filesystem and versioning tool schemas.
4. Enforce storage allowlist, prefix allowlist, read-only override, and TTL.

Out of scope:

1. Persistent session storage.
2. UI for session management.

Required files:

1. `crates/mcp/src/session.rs` (new)
2. `crates/mcp/src/server.rs`
3. `crates/mcp/src/schemas.rs`
4. `crates/mcp/src/tools_fs/common.rs`
5. `crates/mcp/src/tools_fs/*.rs`
6. `crates/mcp/src/errors.rs`
7. `crates/mcp/src/tools_fs/tests.rs`

Acceptance commands:

```bash
cargo fmt --all
cargo clippy -p infimount_mcp --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
cargo test -p infimount_mcp
```

Merge criteria:

1. Out-of-scope access returns `ERR_SESSION_FORBIDDEN`.
2. Missing/expired session returns `ERR_SESSION_NOT_FOUND`.

### 11.5 PR-5: OTEL baseline instrumentation

Title:

1. `feat(mcp): add OTEL tracing and metrics for tool/runtime paths`

In scope:

1. OTEL env var support:
   - `OTEL_EXPORTER_OTLP_ENDPOINT`
   - `OTEL_SERVICE_NAME`
   - `OTEL_RESOURCE_ATTRIBUTES`
2. Tool-level tracing spans.
3. Tool count/error count/latency metrics.
4. Safe no-op behavior when OTEL env vars are absent.

Out of scope:

1. Full enterprise telemetry dashboards.

Required files:

1. `crates/mcp/Cargo.toml`
2. `crates/mcp/src/main.rs`
3. `crates/mcp/src/server.rs`
4. `crates/mcp/src/tools_fs/common.rs` and related helpers
5. `crates/mcp/src/telemetry.rs` (new, recommended)

Acceptance commands:

```bash
cargo fmt --all
cargo clippy -p infimount_mcp --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
cargo test -p infimount_mcp
```

Merge criteria:

1. OTEL-enabled run exports spans/metrics.
2. OTEL-disabled run has no regressions.

### 11.6 PR-6: Desktop versions UI and Tauri wrappers

Title:

1. `feat(desktop): add version history tab and version actions`

In scope:

1. Add versions tab in file preview/detail flows.
2. Call new versioning MCP/Tauri APIs.
3. Add view/download/delete actions for selected version.
4. Show explicit unsupported/not-enabled states.

Out of scope:

1. Admin web UI.
2. Non-version MCP features.

Required files:

1. `apps/desktop/src/components/FilePreviewPanel.tsx`
2. `apps/desktop/src/components/FilePreviewDialog.tsx`
3. `apps/desktop/src/lib/api.ts`
4. `apps/desktop/src/types/storage.ts`
5. `apps/desktop/src-tauri/src/commands.rs`
6. `apps/desktop/src-tauri/src/state.rs`
7. Integration and Playwright tests

Acceptance commands:

```bash
pnpm install --frozen-lockfile
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop test:integration
pnpm --dir apps/desktop test:ui
```

Merge criteria:

1. Versions tab appears only when capability allows.
2. Version actions succeed end-to-end.

### 11.7 PR-7: Final docs and release prep

Title:

1. `docs(release): update MCP/versioning/auth/session/otel user docs`

In scope:

1. Update README and release notes for shipped features.
2. Update operator docs for auth token and OTEL envs.
3. Update changelog.

Acceptance commands:

```bash
pnpm --dir apps/desktop lint
cargo test --workspace
```

Merge criteria:

1. Docs match shipped behavior and error semantics.

## 12. Release Acceptance Checklist

Release is blocked unless all are true:

1. `list_versions/read_file_version/delete_version` available and tested.
2. HTTP auth required unless explicit insecure override.
3. Session scoping enforced across FS and versioning tools.
4. OTEL export works with env vars.
5. Desktop versions UI shipped and gated by capabilities.
6. No secret leakage in logs/UI/responses.
7. Required CI gates green.

## 13. Explicit Out-of-Scope for this release

1. Generic `restore_version`.
2. External DB-backed session/config persistence.
3. Full browser admin UI at `/admin` (optional follow-up).
4. Non-essential adapter servers.

## 14. Agent Working Rules

1. Do not change tool names from this document.
2. Do not weaken schema strictness.
3. Do not bypass capability gates.
4. Do not bypass session checks when `session_id` is provided.
5. Do not introduce secret logging in any code path.
6. Do not merge PRs with failing required workflows.
