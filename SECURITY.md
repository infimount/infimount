# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.8.x   | ✅ |
| < 0.8   | ❌ |

The current stable release is [v0.8.1](https://github.com/infimount/infimount/releases/tag/v0.8.1). Stable packages, updater metadata, checksums, SBOM/provenance, Homebrew metadata, and post-release validation are published from that release line.

## Security Model

Infimount is local-first and does not require an Infimount-hosted backend.

- Storage registry and public backend configuration are stored locally under `~/.infimount/`.
- Storage credentials, OAuth tokens, and the desktop MCP bearer token are stored in the operating system's native secret store.
- JSON configuration stores opaque secret references rather than credential values.
- Infimount fails closed when native secret storage cannot save or verify a secret; current v0.8 releases do not create a new plaintext credential fallback.
- New storages are not exposed to MCP by default. Tool access, storage exposure, path policy, read/write authority, and confirmations are explicit controls.
- The built-in HTTP MCP transport is loopback-only. Remote access requires an authenticated TLS-terminating reverse proxy.
- Recovery backups are age-encrypted and include only configuration plus referenced secret-store values, not remote storage contents.
- Every release requires cryptographically signed updater artifacts. macOS notarization and Windows Authenticode are used only when the corresponding platform-signing credentials are configured, and release notes state the actual signing status.

See the full [Security Model](docs/security.md) for path confinement, confirmation semantics, dependency-audit policy, MCP audit behavior, storage validation, bounded operations, and known local-filesystem race boundaries.

## What To Report

Please report vulnerabilities such as:

- credential disclosure or unauthorized secret-store access
- privilege escalation, path traversal, symlink/reparse-point escape, or unsafe file operations
- remote code execution or command injection
- authentication or authorization bypass for storage or MCP access
- confirmation, session, or policy bypass
- updater/signature/provenance failures
- dependency vulnerabilities with practical impact

## Reporting a Vulnerability

Do not open a public issue for security reports.

Report privately to:

- `security@infimount.org`
- fallback: `rajan.kadeval@gmail.com`

Response targets:

- acknowledgment within 48 hours
- initial triage within 5 business days
- coordinated disclosure after a fix is available

## Non-Production Dummy/Mock Data In Repo

The following files intentionally contain simulator credentials or test/sample data and must not be used in production:

- `storage-simulator/bootstrap.sh`
- `storage-simulator/create_resources.py`
- `storage-simulator/docker-compose.yml`
- `storage-simulator/opendal/s3.yaml`
- `storage-simulator/opendal/webdav.yaml`
- `storage-simulator/opendal/filer.yaml`
- `storage-simulator/opendal/azure.yaml`
- `crates/core/src/bin/verify_storage.rs`

These are local test fixtures only for storage simulators and validation tooling.
