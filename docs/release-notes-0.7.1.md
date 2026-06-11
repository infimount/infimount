# Infimount 0.7.1: Remote File Backends and Mutation Safety

Infimount 0.7.1 adds SFTP and FTP storage support, improves native split-pane browsing, and hardens visible mutation flows so transfers, deletes, and uploads provide safer feedback and stronger automated regression coverage.

Release: <https://github.com/infimount/infimount/releases/tag/v0.7.1>

## Highlights

- Added OpenDAL-backed SFTP and FTP storage backends across desktop Add/Edit Storage, Rust core builders, MCP storage management, capability docs, and tests.
- Reworked split-pane browsing into a same-storage, same-folder file-manager mode with left/right pane labels, a shared header, and a visible close control.
- Fixed recursive folder transfer safety by blocking self-descendant copies and hardening recursive transfer semantics.
- Made upload feedback real: upload progress is tied to actual writes, cancellation stops remaining files, and existing-name conflicts require skip, keep-both, or overwrite.
- Added visible delete progress for long-running deletes.

## MCP and storage safety

- Consolidated OpenDAL operator construction and MCP filesystem operations through `infimount_core` so desktop and MCP behavior stay aligned.
- Preserved MCP policy, session, read-only, and confirmation checks around centralized recursive list, copy, move, and delete flows.
- Rejected unknown storage backends explicitly instead of falling back to local storage.
- Refused destructive storage-root deletes in core.
- Rejected duplicate batch transfer destinations before mutation.
- Continued sanitizing storage errors so backend URLs, query strings, credentials, and file contents are not exposed in UI summaries or MCP output.
- SFTP uses key-based OpenDAL configuration; SFTP password login is not exposed because the OpenDAL SFTP backend does not support it.

## Release and test hardening

- Added Playwright snapshot coverage for split-pane visible actions and delete progress.
- Added regression coverage for upload progress/conflict handling, transfer planning cancellation, recursive transfer behavior, duplicate batch destinations, root delete refusal, MCP policy-aware recursion, and SFTP private-key-path masking.
- Strengthened release automation with post-release validation, release consistency checks, feature-doc drift checks, install-script smoke tests, and zero-manual release gate checks.

## Notes

SFTP and FTP capabilities depend on the configured server and OpenDAL-reported backend capabilities. Use Validate before browsing a new remote file server or exposing it to MCP clients. New/imported storages remain not exposed to MCP by default; agent access remains explicit.
