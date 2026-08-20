# 0005 — Global configuration transaction boundary

Date: 2026-08-19

## Status

Accepted

## Context

Operations that mutate more than one of the storage registry, workspace
registry, MCP settings, app settings, the native secret store, the
restore/import journal, or the MCP runtime must be serialized so a crash or a
concurrent process cannot leave them mutually inconsistent.

There is no single lock that covers all of these stores, so we rely on a fixed
acquisition order to prevent deadlock between processes and between threads.

## Decision

All configuration mutations acquire locks in this exact order:

1. process-local lifecycle mutex (`AppState.lifecycle_mutation`)
2. cross-process configuration transaction lock (fs2 file lock on
   `configuration-transaction.lock` next to the storage registry)
3. workspace mutation lock (workspace registry file lock)
4. storage registry file lock
5. settings/workspace file lock
6. native secret store (keyring) mutation

The configuration transaction lock is exposed by
`StorageRegistry::acquire_configuration_transaction` and is required (as an
RAII guard held for the whole operation) for:

- storage add / update / remove / policy change (desktop commands)
- storage import apply (acquired inside `apply_storage_import_with_validator`)
- backup state snapshot and restore apply/preview (desktop commands)
- MCP auth set / clear / rotate (`apply_mcp_settings_with_auth`)
- workspace create / update / delete when storage policy is involved

Import and restore recovery run only during single-process startup and use the
storage-registry file lock directly; they never acquire the configuration
transaction lock, so there is no inversion.

The desktop UI registers the Tauri single-instance plugin so two desktop
processes cannot run concurrently. The standalone MCP sidecar still relies on
the file locks listed above.

## Consequences

- Two desktop-side mutations are serialized across processes.
- Deadlock is avoided by always acquiring the transaction lock before any
  storage/workspace file lock.
- A process that times out acquiring the transaction lock fails the operation
  with `ERR_REGISTRY_LOCK_TIMEOUT` rather than proceeding unsynchronized.