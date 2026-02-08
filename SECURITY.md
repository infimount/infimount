# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅        |
| < 0.1   | ❌        |

## Security Model

- Infimount is local-first. Storage source definitions are written to `~/.infimount/config.json` (or `INFIMOUNT_CONFIG` if set).
- Credentials are currently stored in that config file as plain JSON values.
- Credentials are **not encrypted at rest** by Infimount in the current release.
- Security relies on OS-level account and filesystem permissions on the host machine.

## What To Report

Please report vulnerabilities such as:

- credential disclosure or unauthorized access
- privilege escalation, path traversal, unsafe file operations
- remote code execution or command injection
- auth bypass against storage backends
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

These are local test fixtures only (for SeaweedFS/Azurite/Fake GCS simulation).
