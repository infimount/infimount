# Zero-manual product coverage gaps

This is the active checklist for replacing manual product testing with automated behavior coverage. A release gate running tests is not enough; each visible storage mutation must have tests that perform the action and assert the user-visible state.

## Covered in automation

- Delete confirmation for keyboard and visible file actions.
- Delete-in-progress panel plus failed-delete retry/cancel-remaining controls with Playwright coverage.
- Transfer queue completion, progress events, retry, sequential queueing, cancellation, planning-state cancellation, and Playwright visible progress/cancel/retry coverage.
- Split-pane open/close same-storage UX with Playwright screenshot coverage.
- Upload progress and upload conflict choices with Playwright screenshot coverage.
- FileBrowser browse into folder, search, list-view switch, empty state, load error, copy/paste conflict resolution, preview open, preview read failure, and download coverage in Playwright.
- Core transfer guards for root delete refusal, self-descendant folder copy, duplicate batch destinations, sanitized storage errors, recursive copy/move/overwrite/rename/skip behavior, and recursive move cancellation source preservation.

## Remaining high-priority gaps

1. Recursive transfer failure semantics.
   - Add core tests for source preservation after backend write/delete failure during move.
2. Simulator-backed product flows.
   - Add desktop/Tauri-level smoke for add/validate/browse/transfer against simulator-backed local/S3/WebDAV-compatible storages where practical.

## Policy direction

Keep `scripts/check-zero-manual-release-gate.sh` focused on verifying gates and the product coverage manifest check in `docs/product-coverage-manifest.json`. The manifest does not pretend coverage is complete; it lists required high-risk flows and blocks release when a flow has no behavioral test file.
