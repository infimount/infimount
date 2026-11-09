# Infimount v0.8.2 C2.8: native hosted-provider credentials

Status: **optional credential configuration only**. Not an Agents API
connection verification, remote execution, data upload, or spending approval.

The committed Agent Workspace Tasks view exposes a small hosted compute
credential panel. The user can enter, replace or explicitly remove an
OpenAI Agents API project key. No key is copied into local configuration JSON,
shareable storage exports, task manifests, model prompts, MCP tools or
provider execution records.

## Trust boundary

- The Tauri command layer stores a validated opaque token under the fixed
  provider-specific OS keyring account
  `openai-agents-api-primary-v1`, using Infimount's existing
  `infimount_core::SecretStore` and `NativeSecretStore`.
- If native OS credential storage is locked/unavailable, saves and deletes
  fail closed. There is **no plaintext fallback**.
- No IPC command exposes the stored token. The read/status endpoint emits
  only `nativeStoreAvailable`, `configured`, and `verified:false`.
  The returned value never includes a token, fingerprint or secret account.
- Commands require operational application state for changes. Keyring
  save/delete operations hold the lifecycle mutation lock so they cannot
  race startup or restore recovery, and execute off the GUI thread. UI
  errors do not echo native errors.
- The UI uses a password field, clears the entered value after an attempted
  save, and requires confirmation to delete a key.
- The key is scoped to the current OS user, not a specific workspace. Like
  other desktop secrets, it exists transiently in app memory during entry.
  Status `configured:true` is **not** a test of provider access or privileges.

## Deliberately not enabled

No code fetches this key to construct an HTTP header or start a paid session.
The existing C2.6 operator-only synthetic CLI continues using the operator's
`OPENAI_API_KEY` environment variable only after explicit authorization.
A future trusted desktop execution flow must retrieve the key solely inside
the native control process and require **separate per-execution**:
reviewed/exported prepared input bytes, provider U.S.-residency and non-ZDR
disclosure, spending consent, bounded session lifecycle and cleanup.
The user must provide a key with project scopes `api.agents.read`,
`api.agents.write` and `api.responses.write`; these permissions cannot
be validated offline.

This credential setup grants **no** authorization to publish outputs to
original storage. The single native human approval chain in #181/#185/#198
remains independent.

Official API-key permissions:
https://developers.openai.com/api/docs/guides/agents-api/quickstart

Current data controls:
https://help.openai.com/en/articles/20001551-agents-api-beta-faq

Stable v0.8.1 remains unchanged.
