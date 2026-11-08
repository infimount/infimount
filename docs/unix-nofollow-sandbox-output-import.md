# Infimount v0.8.2 C1.3c: experimental Unix no-follow output importer

Status: **internal, experimental, Unix-only library code**. No Tauri
command, MCP tool, CLI command, cloud access, source-storage mount, provider
credentials or headless publication. This does not alter v0.8.1 stable.

C1.3a/b proved the create-only output workflow using OpenDAL Memory. This
module exercises a real Local Filesystem destination using pinned Unix file
descriptors. The caller must provide a **trusted** absolute workspace root,
task manifest, separately reviewed redacted export brief, C1.2 candidate
listing and the already downloaded bytes.

The importer:
- Rebuilds the C1.3b reviewed output scope, rejecting missing and extra files.
- Independently SHA-256 hashes and validates *every supplied byte* against
  the candidate metadata **before any destination mutation**.
- Opens each existing root/task/output ancestor one component at a time,
  using `openat(O_DIRECTORY | O_NOFOLLOW)` on pinned descriptors.
- Preflights all exact output leaf paths using
  `fstatat(AT_SYMLINK_NOFOLLOW)`; existing files/symlinks never get overwritten.
- Creates necessary output subdirectories with `mkdirat` and reopens them
  with `O_NOFOLLOW`; creates each file using
  `openat(O_CREAT | O_EXCL | O_NOFOLLOW)`.
- Hashes the committed bytes through the *same pinned file descriptor* after
  writing and syncing.
- Never automatically deletes partial results. If a late write/create/hash
  fails, return `TransferCleanupRequired` and require explicit user-visible
  cleanup before any further execution.
- Runs regression tests on actual temporary files, symlink parent/root escape,
  destination sentinel, altered bytes, duplicate/extra files and a successful
  create-only nested-output ingest.

**Not a production importer yet.** The method is not exported via Tauri or
MCP. A future single trusted invocation must obtain the registry configuration
transaction, workspace mutation locks, session/input/output evidence and
explicit user approval. It must pin the workspace identity and integrate
private execution receipts, interruption/cancellation and cleanup flows.

The primitive belongs to the shared infimount_control crate, not Tauri.
The module is private under cfg(unix), so no desktop, CLI or agent may
invoke it until the trusted control service owns configuration and task locks.

This implementation does not establish nonforgeable same-user-process
authority. macOS/Linux qualification and any Windows reparse-point equivalent
are separate mandatory gates before an execution feature may use this path.

The native publication approval work in #181/#185/#198 remains unresolved.
User-visible output review and original-storage create-only publication
continue to run through their independent existing safe flow.
