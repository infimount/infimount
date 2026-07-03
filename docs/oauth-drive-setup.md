# Guided OAuth for Google Drive and Microsoft OneDrive

Infimount can connect Google Drive and Microsoft OneDrive from the desktop Add Storage dialog. The guided flow opens your browser, receives the provider redirect on a temporary local loopback callback, exchanges the authorization code with PKCE, and saves the resulting tokens only when you save the storage.

## Security model

- The callback listener binds to `127.0.0.1` on a temporary random port.
- The authorization request uses PKCE S256 and validates `state` before token exchange.
- OAuth access tokens, refresh tokens, client secrets, authorization codes, device codes, and PKCE verifiers are treated as secrets.
- Tokens stay in the local Infimount storage registry. Treat `~/.infimount/storages.json` as sensitive.
- MCP exposure remains off by default. Connecting a drive does not expose it to agents.
- Manual token fields remain available as an advanced fallback.

## Google Drive

1. Create or choose a Google Cloud project.
2. Configure an OAuth consent screen for your account or organization.
3. Create an OAuth Client ID for a desktop/native app.
4. In Infimount, choose **Google Drive** in Add Storage.
5. Paste the OAuth Client ID. If your app type/provider setup has a client secret, paste it too.
6. Optionally set a root folder path.
7. Click **Connect Google Drive** and complete browser authorization.
8. Save the storage, then run **Validate** before exposing it to MCP.

Infimount requests Google Drive file access through OpenDAL. Provider policy and account settings can still limit effective capabilities.

## Microsoft OneDrive

1. Register an app in Microsoft Entra / Azure portal.
2. Configure it as a public/native client when possible.
3. Allow loopback redirect behavior for native client OAuth.
4. In Infimount, choose **Microsoft OneDrive** in Add Storage.
5. Paste the OAuth Client ID. Client secret is optional for native/public-client flows.
6. Optionally set a root folder path and enable version listing if your OneDrive account supports it.
7. Click **Connect Microsoft OneDrive** and complete browser authorization.
8. Save the storage, then run **Validate** before exposing it to MCP.

Infimount requests `Files.ReadWrite offline_access`. OneDrive version operations require the `versioning` setting and provider support.

## Validation and smoke testing

Maintainers can run optional live provider smoke tests when intentionally configured with test credentials:

```bash
scripts/oauth-storage-smoke.sh
```

The script skips cleanly when `INFIMOUNT_GDRIVE_*` and `INFIMOUNT_ONEDRIVE_*` variables are absent and does not print secret values.

For zero-manual CI, Infimount uses mocked OAuth callback, token-exchange, UI, and screenshot coverage because Google Drive and OneDrive do not provide a local OpenDAL emulator endpoint.
