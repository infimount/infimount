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

- statements: 47%
- branches: 35%
- functions: 48%
- lines: 50%

## Latest frontend coverage snapshot

Measured with `pnpm test:coverage:frontend`.

Overall:

- statements: 50.93%
- branches: 38.55%
- functions: 52.06%
- lines: 54.26%
- tests: 79 passing

High-confidence areas:

- `FileVersionsTab.tsx`: ~100% line coverage
- `McpSettingsDialog.tsx`: 81.25% line coverage
- `StorageConfigEditorDialog.tsx`: 79.41% line coverage
- `UploadZone.tsx`: 91.89% line coverage
- `api.ts`: 71.31% line coverage
- `mcpNotifications.ts`: 100% line coverage

Low-coverage priority areas:

- `FileTable.tsx`: 28.08% line coverage
- `FileGrid.tsx`: 36.58% line coverage
- `FileBrowser.tsx`: 44.94% line coverage
- `StorageSidebar.tsx`: 42.5% line coverage
- `FilePreviewPanel.tsx`: 48% line coverage
- `JsonCodeEditor.tsx`: 31.25% line coverage
- `use-icon-theme.tsx`: 36.84% line coverage
- `use-toast.ts`: 39.21% line coverage

## Coverage work completed in this pass

- Added tests for notification permission helpers and notification click behavior.
- Raised `mcpNotifications.ts` from partial coverage to 100% line/function/branch coverage.
- Verified frontend coverage still passes threshold.
- Verified Rust coverage gate passes.

## Path to near-zero manual testing

True zero manual testing is unrealistic for a cross-platform desktop storage app, but we can move toward high-confidence automated release testing.

### Phase 1: strengthen core product UI component coverage

Goal: raise frontend line coverage from ~54% to 60%+.

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

Start with `FileTable.tsx` and `FileGrid.tsx` because they are core file-manager surfaces and currently have the lowest coverage. Add tests around context-menu operations and sort/drop behavior first.
