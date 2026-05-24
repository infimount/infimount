# Infimount 0.3.0: Safe MCP Storage Access

## Added

- First-run onboarding for adding storage and finding MCP setup.
- MCP setup and test entry points inside the desktop MCP Settings panel.
- Path-level MCP policies per storage.
- Risky-operation confirmation queue for write, overwrite, delete, version delete, presign, cross-storage copy, and move-style operations.
- MCP audit viewer backed by bounded local audit persistence.
- Version-aware file tools and desktop file-version browsing where backend capabilities allow it.
- MCP session scoping tools for temporary storage/path restrictions.
- Bearer-token enforcement for secure HTTP MCP transport.

## Changed

- Safer MCP operation handling before storage operations reach OpenDAL.
- Clearer exposed-storage, enabled-tool, access-mode, policy-aware access, and capability summaries in MCP Settings.
- Policy prefixes are normalized and de-duplicated before saving.
- Confirmation approvals are single-use and tied to the original request fingerprint.

## Security

- Storage, path, and tool policy is enforced before MCP storage operations execute.
- Destructive or externally exposing operations can require approval in Infimount.
- Confirmation IDs cannot be replayed after approval, denial, expiry, or request tampering.
- Audit log masks presigned URL query parameters and does not store auth tokens, storage secrets, or file contents.
- Pending confirmations are in-memory and are cleared on app/server restart.

## Known Limitations

- Pending confirmations are not restored after restart.
- Desktop notifications are attention signals only; approval still happens inside Infimount.
- Audit persistence is bounded local JSON, not an external tamper-proof audit sink.
- Search/indexing, SFTP/backend expansion, transfer queue, and additional file-operation improvements are deferred.
