# Infimount v0.8.2 C1.4: no-follow verification of prepared inputs

Status: **internal, offline-only, Unix-only**. No Tauri IPC, MCP tool,
remote upload, API credentials, spending or storage publication.

The first hosted adapter accepts inline prepared bytes (up to 32 files,
8 MiB total, 5 MiB per file). The previous browser export preview reports
only manifest-declared hashes. This internal shared-control preflight proves
the **actual prepared bytes** before constructing a provider-specific request.

The new private `sandbox_local_export` module:

1. Validates the user-redacted brief, task manifest, input/output limits and
   exact requested output set using provider-neutral C1 contracts.
2. Opens the trusted workspace root through pinned, no-follow Unix directory
   descriptors. Reopens `tasks/<UUID>/task-manifest.json` as a regular,
   single-linked file and compares its parsed contents with the trusted
   manifest supplied by the caller.
3. Opens each declared prepared input relative to the pinned task root.
   `openat(O_NOFOLLOW|O_NONBLOCK)` prevents symlink and substituted-FIFO
   blocking. Require a single-linked regular file and enforce per-file size.
4. Reads each bounded byte buffer into a `Zeroizing<Vec<u8>>`, independently
   verifies actual SHA-256/size and totals, then passes those exact bytes to
   the already allowlisted `build_openai_hosted_offline_request`.
5. Reuses that request's explicit network-denied, $1 pilot spending ceiling.
   It returns **only an offline request** and immediately zeroizes the
   temporary input buffers, including on errors. The resulting base64
   request still contains reviewed private data and must not be logged or
   returned to browser/MCP tools.
6. Tests verify happy-path request construction, same-size byte substitution,
   manifest mismatch, input leaf/parent symlinks, hardlink aliasing,
   symlinked manifest and missing requested outputs.

This is deliberately private and not production-enabled. The future
trusted command must hold registry configuration and workspace mutation
locks throughout review/transfer, recheck source and workspace identity,
obtain explicit per-export US-residency/non-ZDR and spending consent, fetch
credentials from native secret storage, enforce session lifecycle cleanup
and write private execution evidence before allowing a provider request.
Original storage publication still requires native human approval and
fresh reviewed output hashes. Windows and macOS qualification are separate.
