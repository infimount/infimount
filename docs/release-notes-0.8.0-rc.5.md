# Infimount 0.8.0-rc.5: Security and Release-Policy Candidate

Infimount 0.8.0-rc.5 is the prerelease validation build for the v0.8 recovery, updater trust-chain, and release-integrity work. It supersedes the published rc.4 for validation; rc.4 remains immutable.

Compared with rc.4, rc.5 additionally carries:

- Stable releases may be explicitly platform-unsigned when Apple or Windows signing credentials are unavailable. Apple and Windows signing are evaluated independently; updater signing remains mandatory for every release.
- FTP is temporarily disabled because OpenDAL 0.58.x used `suppaftp` 8.x, affected by [RUSTSEC-2026-0271](https://rustsec.org/advisories/RUSTSEC-2026-0271). The vulnerable `suppaftp` and OpenDAL FTP adapter are removed from the shipped dependency graph.
- Legacy FTP records remain recognizable during migration but fail closed when an operation is attempted. FTP can return after a released OpenDAL version uses `suppaftp >=10.0.2`.
- The dependency audit is green again, with the existing documented RSA exception remaining separately tracked.

Updater artifacts remain cryptographically signed. Checksums, SBOM, provenance, artifact verification, and package-bound MCP sidecar integrity remain mandatory. Release text reports updater signing and each platform's signing status explicitly.

Release: not published yet.

## Validation scope

- Explicitly platform-unsigned macOS and Windows packages, plus Linux packages.
- Clean installation on Linux, macOS, and Windows.
- v0.7.x to v0.8 manual upgrade on all platforms.
- Cross-machine encrypted backup and restore.
- MCP activation through the bundled sidecar with allowed and denied probes.
- Google Drive and OneDrive OAuth storage setup.
- Local symlink and Windows junction denial behavior.
- FTP absence from the dependency graph and safe handling of legacy FTP records.
- The v0.8 updater transition using the corrected signing chain.

This is a prerelease. It must remain marked as a GitHub prerelease and does not update Homebrew or replace the current stable release. Platform signing status is shown explicitly; an unsigned platform package must not be treated as notarized or Authenticode-signed. Use this build only for release validation.

## Upgrade and safety notes

Read [Migrating from v0.7 to v0.8](migration-v0.8.md) before testing an upgrade. Keep MCP exposure explicit, review workspace grants, and create a fresh encrypted recovery backup after migration.

Recovery covers Infimount configuration and referenced credentials, not storage object contents. Backend capabilities remain account- and backend-dependent.
