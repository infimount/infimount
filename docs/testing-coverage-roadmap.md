# Testing Coverage Roadmap

Status: active coverage plan  
Last measured: 2026-05-24

## Current automated gates

### Rust

```bash
cargo test --workspace
./scripts/coverage-rust.sh
```

Current Rust coverage gate:

- `cargo llvm-cov --workspace --fail-under-lines 50`

### Frontend

```bash
cd apps/desktop
pnpm test:unit
pnpm test:integration
pnpm test:ui
pnpm test:coverage:frontend
pnpm lint
pnpm build
```

Current frontend coverage gate:

- statements: 80%
- branches: 65%
- functions: 80%
- lines: 80%

## Latest frontend coverage snapshot

Measured with `pnpm test:coverage:frontend`.

Overall:

- statements: 81.89%
- branches: 67.84%
- functions: 81.93%
- lines: 85.73%
- tests: 139 passing in the frontend coverage suite

High-confidence areas:

- `FileVersionsTab.tsx`: 100% line coverage
- `JsonCodeEditor.tsx`: 100% line coverage
- `api.ts`: 100% line coverage
- `mcpNotifications.ts`: 100% line coverage
- `use-app-zoom.tsx`: 100% line coverage
- `StorageConfigEditorDialog.tsx`: 100% line coverage
- `UploadZone.tsx`: 97.29% line coverage
- `FileIcon.tsx`: 96.42% line coverage
- `FileTable.tsx`: 93.83% line coverage
- `FileGrid.tsx`: 93.9% line coverage
- `use-toast.ts`: 92.15% line coverage
- `AddStorageDialog.tsx`: 88.27% line coverage
- `McpSettingsDialog.tsx`: 83.52% line coverage
- `StorageSidebar.tsx`: 80% line coverage

Remaining priority areas for the next pass:

- `FileBrowser.tsx`: 75.87% line coverage
- `FilePreviewPanel.tsx`: 78% line coverage
- `StorageSidebar.tsx`: 80% line coverage, 73.01% statement coverage
- `McpSettingsDialog.tsx`: 63.51% function coverage

## Coverage work completed in this pass

- Added tests for notification permission helpers and notification click behavior.
- Raised `mcpNotifications.ts` from partial coverage to 100% line/function/branch coverage.
- Added `FileTable` tests for sort callbacks, toggle selection, context-menu actions, internal drag/drop-to-folder behavior, drag payloads, drop rejection paths, and size/date formatting.
- Added `FileGrid` tests for toggle selection, context-menu actions, internal drag/drop-to-folder behavior, drag payloads, drop rejection paths, size formatting, long-name truncation, and themed icon loading.
- Added `FileBrowser` orchestration coverage for search, table/grid switching, sort callback wiring, folder navigation, preview open/edit/download/close, delete confirmation, paste conflicts, uploads, and external file/folder drop collection.
- Added `AddStorageDialog` coverage for add/edit submit paths, validation, secret masking/reveal, reset fields, verification results, and advanced-config preservation.
- Added `FilePreviewPanel`, `FileIcon`, and `use-toast` coverage for preview modes, icon theme loading/cache behavior, and toast reducer/hook dispatch behavior.
- Added drag-selection geometry coverage for `FileTable` and `FileGrid`.
- Added `use-app-zoom` coverage for persisted zoom normalization, shortcut handling, wheel zoom scoping, and provider misuse.
- Added `UploadZone` coverage for drag affordances, directory-relative file picks, imperative uploads, cancellation, and upload completion.
- Added `StorageConfigEditorDialog` coverage for load/reload, formatting errors, save failures, and close behavior.
- Added a release-zero-manual app-shell integration smoke path for adding local storage and starting MCP through the production page orchestration.
- Raised frontend line coverage from 54.26% to 85.73% and installed an 80% line coverage gate.
- Raised frontend statements/functions to 80%+ and increased the coverage gate to 80% statements, 65% branches, 80% functions, and 80% lines.
- Added a `Release` workflow preflight gate that blocks artifact builds until frontend, UI, Rust, desktop smoke, and storage simulator checks pass.
- Added `pnpm test:release` for local release-gate dry runs.
- Verified frontend coverage, lint, unit tests, integration tests, and production build pass.
- Verified Rust coverage gate passes.

## Path to near-zero manual testing

True zero manual testing is unrealistic for a cross-platform desktop storage app, but we can move toward high-confidence automated release testing.

### Phase 1: strengthen core product UI component coverage

Status: first and second targets achieved. Frontend statements, functions, and lines are now above 80%, with enforced gates for each. Branch coverage is now gated at 65%.

Next goal: continue reducing component-level pockets below 80% and work branch coverage toward 80%.

Priority tests:

1. `FileTable.tsx`
   - context menu actions: open/edit/download/delete/cut/copy/paste
   - sort header callbacks
   - folder drag/drop internal transfer
   - drag selection
2. `FileGrid.tsx`
   - context menu actions
   - display-name truncation behavior
   - folder drop target behavior
   - drag selection
3. `FileBrowser.tsx`
   - delete confirmation flow
   - rename/edit text-file flow where applicable
   - view mode switching
   - breadcrumb/path navigation
   - upload/drop error handling
4. `StorageSidebar.tsx`
   - add/edit/delete/verify actions
   - disabled/read-only/exposed badges
   - selected/focused states

### Phase 2: add full-app local-storage E2E

Goal: cover the highest-value manual smoke path.

Proposed scenario:

1. launch desktop app or component-level app shell
2. complete/skip onboarding
3. add local storage pointing at a temp directory
4. browse directory
5. preview text file
6. create folder
7. copy/move/delete test file
8. open MCP settings
9. confirm access summary reflects storage policy

### Phase 3: backend simulator CI

Goal: reduce manual storage backend validation.

Use `storage-simulator` CI to verify:

- S3-compatible read/write/list/delete
- Azure/Azurite read/write/list/delete
- GCS emulator read/write/list/delete
- WebDAV list/read/write
- `validate_storage` capability summary

### Phase 4: accessibility and visual regression

Goal: reduce UI regressions not captured by unit tests.

Add:

- axe checks for dialogs and app shell
- keyboard-only navigation tests
- focus trap checks for dialogs
- screenshot snapshots for main surfaces in light/dark mode

### Phase 5: release/install smoke

Goal: reduce release manual checks.

Automate where possible:

- Linux AppImage launch smoke
- `.deb` and `.rpm` install smoke
- Windows installer smoke in CI
- macOS DMG smoke where runner support allows

## Recommended next coverage task

Focus on `FileBrowser.tsx`, `FilePreviewPanel.tsx`, and `McpSettingsDialog.tsx` branch/function paths. These are now the largest remaining low-coverage areas after the FileTable/FileGrid drag-selection and zoom/storage passes.
