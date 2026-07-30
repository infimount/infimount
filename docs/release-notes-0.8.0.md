# Infimount 0.8.0: Local Trust, Recovery, and Agent Activation

Infimount 0.8.0 focuses on safer local credential handling, explicit agent access, recoverable configuration, and verification of the MCP executable bundled with the desktop app.

Release: <https://github.com/infimount/infimount/releases/tag/v0.8.0>

## Highlights

- **Native secret storage:** persisted storage credentials, OAuth tokens, and desktop MCP bearer tokens use the operating system secret store; JSON configuration keeps opaque references.
- **Encrypted recovery:** create an age-encrypted backup of configuration and referenced secrets, preview changes, and apply a short-lived revision-bound restore with rollback attempts.
- **Agent Workspaces:** create template-backed workspace roots with visible memory files, checkpoint manifests, and exact workspace-managed MCP policy rules.
- **Bundled MCP activation:** the desktop locates a same-version bundled sidecar, runs bounded version/doctor checks, performs a real MCP handshake, and verifies both allowed workspace access and policy denial.
- **Privacy controls:** local diagnostics and product events use bounded stores and sanitized export paths, with explicit telemetry consent.
- **Runtime reliability:** storage operators are revision-keyed and invalidated after configuration changes; directory and preview operations include bounded pagination/range foundations where connected by the backend and caller.

## Security changes

The public MCP surface is data-plane-only. Storage administration stays in the desktop control plane. Fresh and migrated installations use a safe read-only tool baseline, and new storages are not exposed to MCP by default. Non-loopback HTTP requires a bearer token; native secret-store failure does not trigger plaintext fallback.

Stable release tags require macOS signing/notarization material, Windows signing material, and the Tauri updater signing key. Prerelease artifacts may be unsigned only when marked prerelease.

## Upgrade notes

Read [Migrating from v0.7 to v0.8](migration-v0.8.md) before upgrading. Back up `~/.infimount`, allow credential migration to finish, review MCP exposure and tools, then restart external stdio clients. Create a new encrypted recovery backup after migration.

## Known limitations

- Recovery protects Infimount configuration and referenced credentials, not remote object contents.
- Native secret-store operations require an available, unlocked OS user session.
- Backend features such as versions, presigned links, rename, and server-side copy remain backend/account dependent.
- Work across a remote backend and local configuration cannot be one distributed atomic transaction; failures trigger best-effort rollback and sanitized error reporting.

See [Agent Workspaces](agent-workspaces.md), [Recovery](recovery.md), [Privacy](privacy.md), [Troubleshooting](troubleshooting.md), and [Security](security.md) for operational details.
