# Migrating from v0.7 to v0.8

Infimount v0.8 changes credential storage, MCP defaults, workspace persistence, and the desktop-bundled MCP executable. Back up `~/.infimount` before upgrading, without copying it into a public issue or repository.

## Credentials

On startup, v0.8 migrates supported plaintext storage credentials and the desktop MCP bearer token into the operating system's native secret store. JSON configuration keeps opaque secret references rather than credential values. Migration is fail-closed: if the secret store cannot save or verify a value, Infimount reports an error instead of silently discarding it or retaining a new plaintext fallback.

After a successful migration, inspect `storages.json` and `mcp_settings.json` only to confirm they contain references, not to extract secrets. Do not manually rewrite secret references.

## MCP surface and defaults

Storage administration tools are no longer part of public MCP discovery/dispatch. This is an intentional pre-1.0 breaking change; configure storages in the desktop control plane.

Fresh and migrated settings use the safe read-only tool baseline. Existing enabled tools are intersected with that baseline during the security migration. Re-enable mutation tools explicitly after reviewing storage exposure, read-only settings, policies, and confirmation rules. New storages remain unexposed to MCP by default.

Restart stdio clients after upgrade. Desktop-generated client snippets point to the verified bundled sidecar rather than a separately built executable on `PATH`.

## Workspaces

Browser-local workspace metadata moves into `~/.infimount/workspaces.json`. Imported records are normalized and should be reviewed in Agent Workspaces. Workspace access is represented by policy rules named `workspace:<id>`. New workspaces default to read-only access.

## Recovery and rollback

Create a v0.8 encrypted recovery backup after migration succeeds. It includes configuration and referenced native-secret values, not remote storage contents.

If you must return to v0.7, stop MCP clients, restore the pre-upgrade `~/.infimount` backup, and reinstall v0.7. v0.7 does not understand all v0.8 secret references, workspace schema, or policy settings. Do not point both versions at the same live configuration directory.

## Known limitations

- Native secret-store availability depends on an unlocked OS user session.
- Remote storage mutation cannot be made atomically transactional with local registry migration.
- In-memory confirmations and MCP sessions do not survive restart.
- Backend capabilities remain account/server dependent; validate each storage after upgrade.
