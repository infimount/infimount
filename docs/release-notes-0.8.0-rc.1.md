# Infimount 0.8.0-rc.1: Trust and Activation Release Candidate

Infimount 0.8.0-rc.1 is the prerelease validation build for the v0.8 trust, recovery, workspace, and MCP activation work.

Release: not published yet.

## Validation scope

- Native secret storage and transactional plaintext migration.
- Passphrase-encrypted portable recovery for configuration and referenced secrets.
- Rust-owned Agent Workspace creation, policy lifecycle, and bounded checkpoints.
- Same-version bundled-sidecar discovery plus a real allowed/denied MCP activation probe.
- Server-authoritative storage-import preview and apply.
- Revision-aware operator caches, bounded listing, range preview, and streaming transfers.

This is a prerelease. It must remain marked as a GitHub prerelease and does not update Homebrew or replace the current stable release. Prerelease platform applications may omit OS platform signing, but updater artifacts remain cryptographically signed. Unsigned macOS/Windows prerelease builds verify the bundled MCP sidecar against the package-bound checksum; stable builds require platform signatures. Use this build only for release validation.

## Upgrade and safety notes

Read [Migrating from v0.7 to v0.8](migration-v0.8.md) before testing an upgrade. Keep MCP exposure explicit, review workspace grants, and create a fresh encrypted recovery backup after migration.

Recovery covers Infimount configuration and referenced credentials, not storage object contents. Backend capabilities remain account- and backend-dependent.
