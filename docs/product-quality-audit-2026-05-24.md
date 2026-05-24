# Product + Quality Audit

Date: 2026-05-24  
Scope: baseline repository health, product promise alignment, MCP safety, frontend/backend test gates.

## Summary

Baseline quality is healthy. Rust, frontend unit/integration tests, lint, production build, and Playwright component tests pass after installing frontend dependencies. The strongest next product work should focus on making MCP access summaries more exact and continuing to close doc/implementation drift.

## Commands run

From `infimount/`:

```bash
cargo test --workspace
```

Result: passed.

- Tauri crate: 2 passed
- Core crate: 13 passed
- MCP crate: 97 passed
- Doc tests: passed

From `infimount/apps/desktop`:

```bash
pnpm install
pnpm test:unit
pnpm test:integration
pnpm lint
pnpm build
pnpm test:ui
```

Results:

- `pnpm install`: completed; frontend dependencies were previously missing.
- Unit tests: 8 files / 24 tests passed.
- Integration tests: 4 files / 8 tests passed.
- ESLint: passed.
- Production build: passed with bundle-size/browser-data warnings.
- Playwright component tests: 5 passed.

## Expected warnings / observations

### Dependency metadata warnings

Build/test output reports:

- `baseline-browser-mapping` data older than two months.
- Browserslist/caniuse-lite data about six months old.

These are not current failures, but should be handled as maintenance.

### Build output warning

Vite reports:

- `outDir /apps/dist is not inside project root and will not be emptied`.
- Some chunks exceed 500 kB after minification.

These are not current failures, but bundle/code-splitting should become a performance task.

### Secret grep review

Command:

```bash
git grep -nE "(AKIA|BEGIN PRIVATE KEY|AIza|SECRET|TOKEN)"
```

Observed hits are expected references in docs/tests/code constants plus simulator defaults:

- `INFIMOUNT_AUTH_TOKEN` docs/code/tests.
- Storage simulator dummy credentials (`password123`).

No raw production secret was identified in this audit.

## Product surface audit

### Add storage flow

Strengths:

- Backend-specific schemas exist.
- Secret fields are recognized and masked/revealable.
- Validation path exists via backend commands.

Risks/opportunities:

- Backend-specific validation messages should stay human-readable and sanitized.
- Provider presets for S3-compatible services could reduce setup friction later.

### File browser / preview

Strengths:

- Core file operations are delegated through OpenDAL.
- Unit tests cover table/grid/preview behavior.
- Upload and transfer operations exist.

Risks/opportunities:

- Large directory performance and bundle size should become a P1 performance track.
- Keyboard navigation remains a roadmap item and important file-manager baseline.

### MCP settings

Strengths:

- Settings include transport, bind/port, tool exposure, setup snippets, path policies, confirmations, pending approvals, and audit viewer.
- Non-loopback bind warning/confirmation is present.
- The product has a visible “What the agent can access” summary.

Risks/opportunities:

- The access summary is currently high-level. It counts exposed storages and enabled tools, but write/destructive access wording can be more precise by considering storage policies and read-only states, not tool enablement alone.
- HTTP auth token state is primarily backend/settings-level; UI copy should continue to make secure/insecure mode obvious.

### MCP safety backend

Strengths:

- Strong Rust test coverage exists for policy, path normalization, confirmation lifecycle, audit masking, disabled tools, auth requirements, and filesystem tools.
- Denied path prefixes are segment-aware and normalized.
- Confirmation IDs are single-use/fingerprint-bound/expiring.

Risks/opportunities:

- Continue adding integration tests whenever UI behavior changes around policies or confirmations.
- Keep docs aligned with implementation because MCP capabilities changed substantially since earlier docs.

## Prioritized backlog

### P0 — trust/docs consistency

1. **Make MCP access summary policy-aware.** — Completed in this audit pass.
   - The summary now accounts for exposed storages, `readOnly`, and `mcpPolicy.default_access` before implying write/destructive/presign access.
   - A frontend integration test covers read-only/no-access policy behavior.
   - User value: clearer, safer understanding before connecting an agent.

2. **Finish docs drift cleanup.**
   - `Agents.md` was updated for current backend support.
   - Continue reviewing docs for stale claims around MCP vNext vs implemented behavior.

### P1 — file-manager fundamentals

3. **Keyboard navigation pass.**
   - Sidebar, file table/grid, dialogs, and preview should be navigable and visibly focused.

4. **Large-directory and bundle performance pass.**
   - Investigate Vite chunk warnings.
   - Consider manual chunks and lazy loading around large icon themes/editor dependencies.

### P1 — MCP UX clarity

5. **Improve MCP summary detail.**
   - List exposed storages by access class: no access, read-only, read/write.
   - Show if risky operations require confirmation.
   - Show if any destructive tool is enabled against any write-capable storage.

6. **Audit viewer filtering/export.**
   - Add filters by storage/tool/decision while preserving secret-safe audit behavior.

### P2 — maintenance

7. **Refresh browser metadata dependencies.**
   - Update browserslist/baseline metadata in a controlled dependency-maintenance PR.

8. **Review Vite `outDir` warning.**
   - Decide if the current `apps/dist` location is intentional for Tauri.
   - Configure `emptyOutDir` or document why not.

## Recommended next engineering task

Next task: **continue docs drift cleanup around MCP vNext and implemented behavior**.

Acceptance criteria:

- Public docs describe implemented behavior, not stale planning state.
- MCP security docs, setup docs, backend-capability docs, and agent docs agree on supported backends and safety controls.
- Any deferred features remain clearly marked as future/planned.
