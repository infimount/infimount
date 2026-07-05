# Infimount 0.7.2: Guided OAuth for Drive Storage

Infimount 0.7.2 adds guided desktop OAuth connection for Google Drive and Microsoft OneDrive while preserving the local-first storage model and explicit MCP opt-in behavior.

Release: <https://github.com/infimount/infimount/releases/tag/v0.7.2>

## Highlights

- Added guided OAuth connection for Google Drive and Microsoft OneDrive from the Add/Edit Storage dialog.
- Uses a local loopback callback on `127.0.0.1` with a temporary random port.
- Uses PKCE S256 and OAuth `state` validation before token exchange.
- Stores OAuth tokens locally only after the user saves the storage.
- Keeps manual token fields available as an advanced fallback.
- Keeps MCP exposure opt-in; connecting a drive does not expose it to MCP clients by default.

## Security and validation

- OAuth access tokens, refresh tokens, client secrets, authorization codes, device codes, and PKCE verifiers remain treated as secrets and are masked from UI text, logs, validation summaries, audit exports, and token-exchange errors.
- Callback handling rejects wrong paths and non-loopback peers, times out stalled callback reads, and closes after the first terminal response.
- Added mocked Google Drive and Microsoft OneDrive token-exchange coverage, callback state/timeout/path coverage, frontend unit coverage, and Playwright screenshot coverage for OAuth connect states.
- Live Google Drive and Microsoft OneDrive provider smoke remains credential-gated through `scripts/oauth-storage-smoke.sh`; it is skipped when provider credentials are not intentionally supplied.

## Notes

Infimount stores OAuth tokens in the local storage registry. This release does not add encrypted or OS-keychain token storage. Google and Microsoft provider verification status depends on the OAuth app configured by the user or organization. Use Validate before relying on a new cloud-drive storage or exposing it to MCP clients.
