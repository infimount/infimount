# Troubleshooting

## Activation cannot find or verify the MCP sidecar

The desktop release bundles a same-version `infimount_mcp` executable. Activation does not use an arbitrary executable from `PATH`.

From a source checkout, run:

```bash
node scripts/prepare-mcp-sidecar.mjs
bash scripts/smoke-mcp-sidecar.sh
```

A packaged sidecar supports:

```bash
/path/to/infimount_mcp --version
/path/to/infimount_mcp doctor --json
```

`doctor --json` reports canonical configuration status without absolute local paths. A malformed `storages.json` or `mcp_settings.json` is unhealthy; missing files on a clean installation are reported as not configured.

Activation requires an enabled, MCP-exposed storage with a workspace policy rule. It starts the bundled sidecar temporarily, negotiates MCP, performs an allowed workspace list, and proves an out-of-scope request returns `ERR_MCP_POLICY_DENIED`. Fix the reported step before completing onboarding.

## HTTP MCP does not start

- Prefer `127.0.0.1`.
- Non-loopback HTTP requires a bearer token in the native secret store.
- If the secret store is locked or unavailable, unlock it and retry; Infimount does not fall back to a plaintext token.
- After token rotation, reconnect clients with the new token.

Never include the token in screenshots or diagnostics.

## Storage validation fails

Use the sanitized validation category and fix hints. Confirm endpoint, bucket/container/root, permissions, and backend-specific capability in [Backend Capability Matrix](backend-capabilities.md). Validation does not enable MCP exposure automatically.

## Workspace creation or restore fails

Workspace roots cannot be the storage root, overlap another workspace, or contain traversal/control segments. Existing non-empty roots require explicit adoption. A recovery apply requires the still-valid preview ID and unchanged local state; preview again after any configuration change.

## Diagnostics

Use **Diagnostics** to view local status and export a redacted support bundle. Review the bundle before sharing it. It excludes credential values and file contents by design; see [Privacy and Diagnostics](privacy.md).

## Unsigned prerelease warning

Every release requires signed updater artifacts. Platform app signing is included when credentials are configured; this project may publish explicitly platform-unsigned stable or prerelease applications, so operating-system warnings can occur. Never treat an unsigned application as notarized or Authenticode-signed. On an unsigned prerelease only, Infimount runs its bundled MCP sidecar only when the package-bound `mcp.sha256` matches; signed sidecars prefer platform signature verification. Stable builds never use this checksum as a substitute for macOS code signing or Windows Authenticode. An updater artifact without a valid cryptographic signature is never accepted. Do not treat an unsigned prerelease application as a stable release.
