# Zero-manual product coverage gaps

This is the active checklist for replacing manual product testing with automated behavior coverage. A release gate running tests is not enough; each visible storage mutation must have tests that perform the action and assert the user-visible state.

## Covered in automation

- Delete confirmation for keyboard and visible file actions.
- Delete-in-progress panel with Playwright screenshot coverage.
- Transfer queue completion, progress events, retry, sequential queueing, cancellation, planning-state cancellation, and Playwright visible progress/cancel/retry coverage.
- Split-pane open/close same-storage UX with Playwright screenshot coverage.
- Upload progress and upload conflict choices with Playwright screenshot coverage.
- FileBrowser browse into folder, search, list-view switch, empty state, and load error coverage in Playwright.
- Core transfer guards for root delete refusal, self-descendant folder copy, duplicate batch destinations, sanitized storage errors, and basic copy/move/rename/skip.

## Remaining high-priority gaps

1. Delete cancel/retry.
   - Current delete progress is visible but cannot cancel remaining queued selected items or retry failed items from the panel.
2. Recursive transfer semantics.
   - Add core tests for recursive folder copy/move success, overwrite removing stale destination children, skip leaving destination untouched, folder rename, cancellation during recursive move, and source preservation after failed move.
3. Transfer queue conflict-resolution Playwright coverage.
   - Add screen-level tests for conflict resolution from a FileBrowser flow.
4. Preview/download Playwright coverage.
   - Add real FileBrowser-to-FilePreviewPanel tests for preview open, download failure/success, and preview error states.
5. Simulator-backed product flows.
   - Add desktop/Tauri-level smoke for add/validate/browse/transfer against simulator-backed local/S3/WebDAV-compatible storages where practical.

## Policy direction

Keep `scripts/check-zero-manual-release-gate.sh` focused on verifying gates and the product coverage manifest check in `docs/product-coverage-manifest.json`. The manifest does not pretend coverage is complete; it lists required high-risk flows and blocks release when a flow has no behavioral test file.
