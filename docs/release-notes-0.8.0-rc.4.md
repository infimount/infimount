# Infimount 0.8.0-rc.4: Release-Preflight and Recovery-Ordering Candidate

Infimount 0.8.0-rc.4 is the prerelease validation build for the v0.8 release-preflight, recovery-ordering, and updater trust-chain work.

Compared with rc.3 (which never published), rc.4 additionally carries:

- One-lock startup recovery ordering: restore recovery runs first under a single cross-process configuration lock, followed by import recovery, secret-reference transaction recovery, secret cleanup, plaintext/legacy cleanup, and migration cleanup. The mutation path checks restore state before any other mutation.
- The standalone MCP sidecar fails closed while a desktop restore journal is pending.
- Updater signing secrets are scoped to the exact release steps that need them: compilation and third-party setup actions run without secrets, key correspondence is verified by a pre-built derivation binary before platform builds, and publishing uses only the public key.
- Release preflight now verifies updater private/public key correspondence before any platform build starts.
- The release workflow defaults to read-only GitHub token permissions at workflow scope.
- Fixes the updater signing-key correspondence preflight so equivalent minisign public-key text is compared after newline normalization.
- v0.7.x to v0.8 remains a one-time manual upgrade because of the updater trust-chain correction introduced in rc.2.

Release: not published yet.

## Validation scope

- Production-signed macOS and Windows packages where credentials are configured.
- Clean installation on Linux, macOS, and Windows.
- v0.7.x to v0.8 manual upgrade on all platforms.
- Cross-machine encrypted backup and restore.
- MCP activation through the bundled sidecar with allowed and denied probes.
- Google Drive and OneDrive OAuth storage setup.
- Local symlink and Windows junction denial behavior.
- The v0.8 updater transition using the corrected signing chain.

This is a prerelease. It must remain marked as a GitHub prerelease and does not update Homebrew or replace the current stable release. Prerelease platform applications may omit OS platform signing, but updater artifacts remain cryptographically signed. Use this build only for release validation.

## Upgrade and safety notes

Read [Migrating from v0.7 to v0.8](migration-v0.8.md) before testing an upgrade. Keep MCP exposure explicit, review workspace grants, and create a fresh encrypted recovery backup after migration.

Recovery covers Infimount configuration and referenced credentials, not storage object contents. Backend capabilities remain account- and backend-dependent.
