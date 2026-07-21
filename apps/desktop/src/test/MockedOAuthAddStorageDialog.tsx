import { AddStorageDialog } from "@/components/AddStorageDialog";
import type { OAuthConnectInput, OAuthConnectResult, StorageKindSchema } from "@/lib/api";
import type { StorageConfig, StorageDraft, StorageValidationResult } from "@/types/storage";

const googleDriveSchema: StorageKindSchema = {
  id: "google-drive",
  label: "Google Drive",
  kind: "gdrive",
  fields: [
    { name: "rootPath", label: "Root Folder Path", input_type: "text" },
    { name: "accessToken", label: "Access Token", input_type: "password", secret: true },
    { name: "refreshToken", label: "Refresh Token", input_type: "password", secret: true },
    { name: "clientId", label: "OAuth Client ID", input_type: "text", secret: true },
    { name: "clientSecret", label: "OAuth Client Secret", input_type: "password", secret: true },
  ],
};

const oneDriveSchema: StorageKindSchema = {
  id: "onedrive",
  label: "Microsoft OneDrive",
  kind: "onedrive",
  fields: [
    { name: "rootPath", label: "Root Folder Path", input_type: "text" },
    { name: "accessToken", label: "Access Token", input_type: "password", secret: true },
    { name: "refreshToken", label: "Refresh Token", input_type: "password", secret: true },
    { name: "clientId", label: "OAuth Client ID", input_type: "text", secret: true },
    { name: "clientSecret", label: "OAuth Client Secret", input_type: "password", secret: true },
    { name: "versioning", label: "Enable Versioning", input_type: "checkbox" },
  ],
};

const validationResult: StorageValidationResult = {
  valid: true,
  details: "Storage validated successfully.",
  capabilities: {
    list: true,
    stat: true,
    read: true,
    write: true,
    delete: true,
    copy: false,
    rename: false,
    presign_read: false,
    create_dir: true,
    write_with_user_metadata: false,
    list_with_versions: false,
    read_with_version: false,
    delete_with_version: false,
  },
  fix_hints: [],
  warnings: ["Storage is not exposed to MCP clients."],
};

const policy: StorageConfig["mcpPolicy"] = {
  version: 2,
  default_access: "read_only",
  rules: [],
  allowed_paths: [],
  denied_paths: [],
  confirmation_rules: {
    require_for_write: true,
    require_for_overwrite: true,
    require_for_delete: true,
    require_for_version_delete: true,
    require_for_presign: true,
    require_for_cross_storage_copy: true,
  },
};

function initialStorage(provider: "gdrive" | "onedrive"): StorageConfig {
  return {
    id: `${provider}-visual`,
    name: provider === "gdrive" ? "Google Drive" : "Microsoft OneDrive",
    type: provider === "gdrive" ? "google-drive" : "onedrive",
    backend: provider,
    config: {
      clientId: provider === "gdrive" ? "google-client-id.apps.googleusercontent.com" : "onedrive-client-id",
      clientSecret: "client-secret-kept-local",
      rootPath: provider === "gdrive" ? "/Infimount" : "/Projects",
      ...(provider === "onedrive" ? { versioning: true } : {}),
    },
    enabled: true,
    mcpExposed: false,
    readOnly: false,
    connected: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    mcpPolicy: policy,
  };
}

export function MockedOAuthAddStorageDialog({
  provider = "gdrive",
  mode = "success",
}: {
  provider?: "gdrive" | "onedrive";
  mode?: "success" | "waiting" | "error";
}) {
  const schema = provider === "gdrive" ? googleDriveSchema : oneDriveSchema;
  const connectOAuth = async (input: OAuthConnectInput): Promise<OAuthConnectResult> => {
    (window as Window & { __PLAYWRIGHT_OAUTH_INPUT__?: OAuthConnectInput }).__PLAYWRIGHT_OAUTH_INPUT__ = input;

    if (mode === "waiting") {
      return new Promise(() => undefined);
    }

    if (mode === "error") {
      throw new Error("OAuth authorization failed without exposing tokens.");
    }

    return {
      provider,
      oauthSessionId: "playwright-oauth-session",
      publicConfig: {
        clientId: input.clientId,
        rootPath: input.rootPath,
        ...(provider === "onedrive" ? { versioning: input.versioning ?? false } : {}),
      },
      expiresAt: "2026-01-01T00:10:00Z",
    };
  };

  return (
    <AddStorageDialog
      open
      onOpenChange={() => undefined}
      loadSchemas={async () => [schema]}
      initialStorage={initialStorage(provider)}
      connectOAuth={connectOAuth}
      onVerify={async () => validationResult}
      onUpdate={async (_id: string, draft: StorageDraft) => {
        (window as Window & { __PLAYWRIGHT_OAUTH_UPDATE__?: StorageDraft }).__PLAYWRIGHT_OAUTH_UPDATE__ = draft;
      }}
    />
  );
}
