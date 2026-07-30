# Recovery Backup and Restore

Infimount v0.8 can create an encrypted recovery backup of local configuration and the secret-store values referenced by that configuration.

## Create a backup

Open the Recovery Backup dialog, choose a destination file, and enter the passphrase twice. The passphrase is kept in component memory only for the operation and is not saved. The backup is age-encrypted and includes a checksum-protected, versioned payload.

Backup creation fails closed if a persisted storage or MCP setting references a secret that cannot be read from the native secret store. A successful backup may contain usable credentials inside its encrypted payload, so protect both the backup file and passphrase.

## Restore

Restore has two separate steps:

1. **Preview** decrypts and strictly validates the payload, reports additions/replacements, and creates a short-lived preview ID bound to the current local configuration state.
2. **Apply** consumes that preview ID once. It refuses an expired, reused, or state-stale preview.

Apply snapshots affected registry, settings, workspace, and secret-store state. If persistence or runtime reconciliation fails, Infimount attempts rollback and reports sanitized rollback error codes. After apply or rollback it clears cached storage operators and reconciles the desktop HTTP MCP runtime.

Do not edit configuration between preview and apply. If it changes, create a new preview.

## Passphrase and key handling

- Infimount does not store the backup passphrase.
- A lost passphrase cannot be recovered by Infimount.
- Do not paste passphrases into support bundles, logs, or issue reports.
- Keep backups off shared or world-readable paths.

## Clean-machine recovery

Install the same or newer compatible Infimount release, open Recovery Backup, preview the backup, and verify the reported changes before applying. Restart external stdio MCP clients after restore so they reload configuration. Desktop HTTP is reconciled automatically, but clients may need to reconnect.

## Limits

Recovery backs up Infimount configuration and referenced secrets, not storage object contents. Remote files, bucket versions, provider IAM policy, operating-system keychain policy, and active in-memory MCP confirmations/sessions are outside the backup.
