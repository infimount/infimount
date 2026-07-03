import { useEffect, useMemo, useState } from "react";
import { Clipboard, ExternalLink, Eye, EyeOff, Sparkles } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  connectOAuthStorage,
  listStorageSchemas,
  type OAuthConnectInput,
  type OAuthConnectResult,
  type StorageFieldSchema,
  type StorageKindSchema,
} from "@/lib/api";
import type {
  StorageConfig,
  StorageDraft,
  StorageType,
  StorageValidationResult,
} from "@/types/storage";
import s3Icon from "@/assets/amazon-s3.svg";
import azureIcon from "@/assets/azure-storage-blob.svg";
import gcsIcon from "@/assets/google-cloud.svg";
import webdavIcon from "@/assets/webdav.svg";
import folderNetworkIcon from "@/assets/folder-network.svg";

const STORAGE_TYPE_ICONS: Record<string, string> = {
  "aws-s3": s3Icon,
  "backblaze-b2": folderNetworkIcon,
  "aliyun-oss": folderNetworkIcon,
  "tencent-cos": folderNetworkIcon,
  "huawei-obs": folderNetworkIcon,
  "azure-blob": azureIcon,
  gcs: gcsIcon,
  "google-drive": gcsIcon,
  onedrive: folderNetworkIcon,
  sftp: folderNetworkIcon,
  ftp: folderNetworkIcon,
  webdav: webdavIcon,
  "local-fs": folderNetworkIcon,
};

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

interface ProviderPreset {
  id: string;
  label: string;
  description: string;
  defaultName: string;
  values: Record<string, string>;
}

const PROVIDER_PRESETS: Partial<Record<StorageType, ProviderPreset[]>> = {
  "aws-s3": [
    {
      id: "cloudflare-r2",
      label: "Cloudflare R2",
      description: "S3-compatible object storage. Replace the account id in the endpoint.",
      defaultName: "Cloudflare R2",
      values: {
        region: "auto",
        endpoint: "https://<account-id>.r2.cloudflarestorage.com",
      },
    },
    {
      id: "minio",
      label: "MinIO",
      description: "Local or self-hosted S3-compatible storage.",
      defaultName: "MinIO",
      values: {
        region: "us-east-1",
        endpoint: "http://localhost:9000",
      },
    },
    {
      id: "wasabi",
      label: "Wasabi",
      description: "Replace the region if your bucket lives outside us-east-1.",
      defaultName: "Wasabi Bucket",
      values: {
        region: "us-east-1",
        endpoint: "https://s3.us-east-1.wasabisys.com",
      },
    },
    {
      id: "backblaze-b2",
      label: "Backblaze B2",
      description: "S3-compatible endpoint. Replace the region with your bucket region.",
      defaultName: "Backblaze B2",
      values: {
        region: "us-west-004",
        endpoint: "https://s3.us-west-004.backblazeb2.com",
      },
    },
    {
      id: "digitalocean-spaces",
      label: "DigitalOcean Spaces",
      description: "S3-compatible Spaces endpoint. Replace nyc3 when needed.",
      defaultName: "DigitalOcean Space",
      values: {
        region: "nyc3",
        endpoint: "https://nyc3.digitaloceanspaces.com",
      },
    },
  ],
  webdav: [
    {
      id: "nextcloud",
      label: "Nextcloud",
      description: "WebDAV URL for a user's files. Replace the host and username.",
      defaultName: "Nextcloud Files",
      values: {
        serverUrl: "https://cloud.example.com/remote.php/dav/files/<username>/",
        rootPath: "/",
      },
    },
  ],
};

interface AddStorageDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd?: (config: StorageDraft) => Promise<void>;
  onUpdate?: (id: string, config: StorageDraft) => Promise<void>;
  onVerify?: (config: StorageDraft) => Promise<StorageValidationResult>;
  initialStorage?: StorageConfig;
  loadSchemas?: () => Promise<StorageKindSchema[]>;
  connectOAuth?: (input: OAuthConnectInput) => Promise<OAuthConnectResult>;
}

const DEFAULT_TYPE: StorageType = "local-fs";

export function AddStorageDialog({
  open,
  onOpenChange,
  onAdd,
  onUpdate,
  onVerify,
  initialStorage,
  loadSchemas = listStorageSchemas,
  connectOAuth = connectOAuthStorage,
}: AddStorageDialogProps) {
  const isEditing = Boolean(initialStorage);
  const [schemas, setSchemas] = useState<StorageKindSchema[]>([]);
  const [name, setName] = useState("");
  const [type, setType] = useState<StorageType>(DEFAULT_TYPE);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [extraConfig, setExtraConfig] = useState<Record<string, unknown>>({});
  const [enabled, setEnabled] = useState(true);
  const [mcpExposed, setMcpExposed] = useState(false);
  const [readOnly, setReadOnly] = useState(false);
  const [revealSecrets, setRevealSecrets] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isVerifying, setIsVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<StorageValidationResult | null>(null);
  const [verifyMessage, setVerifyMessage] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [oauthStatus, setOauthStatus] = useState<string | null>(null);
  const [isOAuthConnecting, setIsOAuthConnecting] = useState(false);

  useEffect(() => {
    let mounted = true;
    loadSchemas()
      .then((items) => {
        if (!mounted) return;
        setSchemas(items);
      })
      .catch((error) => {
        console.error("Failed to load storage schemas", error);
      });

    return () => {
      mounted = false;
    };
  }, [loadSchemas]);

  const currentSchema = useMemo(
    () => schemas.find((schema) => schema.id === type),
    [schemas, type],
  );

  const hasSecretFields = useMemo(
    () => currentSchema?.fields.some((field) => field.secret) ?? false,
    [currentSchema],
  );

  useEffect(() => {
    if (!open) {
      setFormError(null);
      setVerifyResult(null);
      setVerifyMessage(null);
      setIsSubmitting(false);
      setIsVerifying(false);
      setCopyStatus(null);
      setOauthStatus(null);
      setIsOAuthConnecting(false);
      return;
    }

    if (schemas.length === 0) return;

    if (initialStorage) {
      const nextType = initialStorage.type;
      const schema = schemas.find((item) => item.id === nextType);
      const knownFieldNames = new Set(schema?.fields.map((field) => field.name) ?? []);
      const nextFieldValues = buildFieldValues(schema, initialStorage.config);
      const preservedConfig = Object.fromEntries(
        Object.entries(initialStorage.config).filter(([key]) => !knownFieldNames.has(key)),
      );

      setName(initialStorage.name);
      setType(nextType);
      setFieldValues(nextFieldValues);
      setExtraConfig(preservedConfig);
      setEnabled(initialStorage.enabled);
      setMcpExposed(initialStorage.mcpExposed);
      setReadOnly(initialStorage.readOnly);
      setRevealSecrets(
        !(schema?.fields.some((field) => field.secret && nextFieldValues[field.name]) ?? false),
      );
      return;
    }

    const schema = schemas.find((item) => item.id === DEFAULT_TYPE) ?? schemas[0];
    const nextType = (schema?.id ?? DEFAULT_TYPE) as StorageType;

    setName("");
    setType(nextType);
    setFieldValues(buildFieldValues(schema));
    setExtraConfig({});
    setEnabled(true);
    setMcpExposed(false);
    setReadOnly(false);
    setRevealSecrets(true);
  }, [initialStorage, open, schemas]);

  const handleTypeChange = (value: StorageType) => {
    const schema = schemas.find((item) => item.id === value);
    setType(value);
    setFieldValues(buildFieldValues(schema));
    setExtraConfig({});
    setRevealSecrets(true);
    setFormError(null);
    setVerifyResult(null);
    setVerifyMessage(null);
    setOauthStatus(null);
  };

  const handleFieldChange = (fieldName: string, value: string) => {
    setFieldValues((current) => ({
      ...current,
      [fieldName]: value,
    }));
    setFormError(null);
    setVerifyResult(null);
    setVerifyMessage(null);
  };

  const applyTemplate = () => {
    setFieldValues(buildFieldValues(currentSchema));
    setExtraConfig({});
    setFormError(null);
    setRevealSecrets(true);
  };

  const applyProviderPreset = (preset: ProviderPreset) => {
    setFieldValues((current) => ({
      ...current,
      ...preset.values,
    }));
    setName((current) => current.trim() || preset.defaultName);
    setFormError(null);
    setVerifyResult(null);
    setVerifyMessage(null);
  };

  const providerPresets = PROVIDER_PRESETS[type] ?? [];
  const isOAuthStorage = type === "google-drive" || type === "onedrive";
  const oauthProvider = type === "google-drive" ? "gdrive" : type === "onedrive" ? "onedrive" : null;
  const oauthConnected = Boolean(fieldValues.accessToken || fieldValues.refreshToken);

  const handleOAuthConnect = async () => {
    if (!oauthProvider) return;
    const clientId = (fieldValues.clientId ?? "").trim();
    if (!clientId) {
      setFormError("OAuth Client ID is required before connecting.");
      return;
    }

    setIsOAuthConnecting(true);
    setFormError(null);
    setOauthStatus("Opening your browser for local OAuth authorization...");
    try {
      const result = await connectOAuth({
        provider: oauthProvider,
        clientId,
        clientSecret: (fieldValues.clientSecret ?? "").trim() || undefined,
        rootPath: (fieldValues.rootPath ?? "").trim() || undefined,
        versioning: fieldValues.versioning === "true",
      });
      const nextValues = buildFieldValues(currentSchema, { ...fieldValues, ...result.config });
      setFieldValues(nextValues);
      setRevealSecrets(false);
      setOauthStatus("OAuth connected. Tokens are stored locally when you save this storage.");
      setName((current) => current.trim() || (oauthProvider === "gdrive" ? "Google Drive" : "Microsoft OneDrive"));
    } catch (error) {
      setOauthStatus(null);
      setFormError(error instanceof Error ? error.message : "OAuth authorization failed.");
    } finally {
      setIsOAuthConnecting(false);
    }
  };

  const buildDraft = (): StorageDraft | null => {
    if (!currentSchema) {
      setFormError("Storage schema is not available yet.");
      return null;
    }

    const trimmedName = name.trim();
    if (!trimmedName) {
      setFormError("Storage Name is required.");
      return null;
    }

    const config: Record<string, unknown> = { ...extraConfig };

    for (const field of currentSchema.fields) {
      const rawValue = fieldValues[field.name] ?? "";
      if (field.required && !rawValue.trim()) {
        setFormError(`${field.label} is required.`);
        return null;
      }

      if (field.input_type === "checkbox") {
        config[field.name] = rawValue === "true";
        continue;
      }

      if (!rawValue.trim()) continue;
      config[field.name] = rawValue;
    }

    setFormError(null);
    return {
      name: trimmedName,
      backend: mapStorageTypeToBackend(type),
      config,
      enabled,
      mcpExposed,
      readOnly,
    };
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    const draft = buildDraft();
    if (!draft) return;

    setIsSubmitting(true);
    try {
      if (isEditing && initialStorage && onUpdate) {
        await onUpdate(initialStorage.id, draft);
      } else if (onAdd) {
        await onAdd(draft);
      }
      onOpenChange(false);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleVerify = async () => {
    if (!onVerify) return;
    const draft = buildDraft();
    if (!draft) return;

    setIsVerifying(true);
    setVerifyMessage(null);
    setCopyStatus(null);
    try {
      const result = await onVerify(draft);
      setVerifyResult(result);
      setVerifyMessage(result.valid ? "Storage validated successfully." : result.details);
    } catch (error) {
      setVerifyResult(null);
      setVerifyMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setIsVerifying(false);
    }
  };

  const handleCopyValidationSummary = async () => {
    if (!verifyResult) return;
    try {
      await navigator.clipboard.writeText(buildValidationSummary(verifyResult));
      setCopyStatus("Validation summary copied.");
    } catch {
      setCopyStatus("Could not copy validation summary.");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[720px] max-h-[88vh] overflow-y-auto rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-base font-normal text-[hsl(var(--card-foreground))]">
            {isEditing ? "Edit Storage" : "Add New Storage"}
          </DialogTitle>
          <DialogDescription className="text-left text-xs text-muted-foreground">
            Configure the storage with the guided form. Full registry JSON editing is available from
            the storage menu.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="mt-2 space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="storage-name" className="text-xs font-normal text-muted-foreground">
                Storage Name
              </Label>
              <Input
                id="storage-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Research Bucket"
                required
                className={`border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="storage-type" className="text-xs font-normal text-muted-foreground">
                Storage Type
              </Label>
              <Select
                value={type}
                onValueChange={(value) => handleTypeChange(value as StorageType)}
              >
                <SelectTrigger
                  id="storage-type"
                  className={`border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] focus:border-ring focus:ring-0 focus:ring-offset-0 data-[state=open]:border-ring ${FIELD_FOCUS_CLASS}`}
                >
                  <SelectValue>
                    {(() => {
                      const current = schemas.find((schema) => schema.id === type);
                      const icon = STORAGE_TYPE_ICONS[type];
                      if (!current) return null;
                      return (
                        <span className="flex items-center gap-2">
                          {icon ? (
                            <img src={icon} alt="" aria-hidden="true" className="h-4 w-4" />
                          ) : null}
                          <span>{current.label}</span>
                        </span>
                      );
                    })()}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                  {schemas.map((schema) => (
                    <SelectItem
                      key={schema.id}
                      value={schema.id}
                      className="focus:bg-sidebar-accent/40 focus:text-sidebar-foreground"
                    >
                      <div className="flex items-center gap-2">
                        {STORAGE_TYPE_ICONS[schema.id] ? (
                          <img
                            src={STORAGE_TYPE_ICONS[schema.id]}
                            alt=""
                            aria-hidden="true"
                            className="h-4 w-4"
                          />
                        ) : null}
                        <span>{schema.label}</span>
                      </div>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid gap-3 rounded-xl border border-border/70 bg-card/40 p-4 md:grid-cols-3">
            <ToggleRow
              id="storage-enabled"
              label="Enabled"
              description="Available in the desktop app."
              checked={enabled}
              onCheckedChange={setEnabled}
            />
            <ToggleRow
              id="storage-mcp-exposed"
              label="Expose to MCP"
              description="Visible from the MCP virtual root."
              checked={mcpExposed}
              onCheckedChange={setMcpExposed}
            />
            <ToggleRow
              id="storage-read-only"
              label="Read-only"
              description="Blocks writes, deletes, and moves."
              checked={readOnly}
              onCheckedChange={setReadOnly}
            />
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <Label className="text-xs font-normal text-muted-foreground">Backend Fields</Label>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  Use the guided form for the selected backend configuration.
                </p>
              </div>
              <div className="flex flex-wrap items-center justify-end gap-2">
                <Button
                  type="button"
                  variant="outline"
                  className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
                  onClick={applyTemplate}
                >
                  <Sparkles className="mr-2 h-4 w-4" />
                  Reset Fields
                </Button>
                {hasSecretFields ? (
                  <Button
                    type="button"
                    variant="outline"
                    className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
                    onClick={() => setRevealSecrets((current) => !current)}
                  >
                    {revealSecrets ? (
                      <>
                        <EyeOff className="mr-2 h-4 w-4" />
                        Mask Secrets
                      </>
                    ) : (
                      <>
                        <Eye className="mr-2 h-4 w-4" />
                        Reveal Secrets
                      </>
                    )}
                  </Button>
                ) : null}
              </div>
            </div>

            {providerPresets.length > 0 ? (
              <div className="rounded-xl border border-border/70 bg-card/40 p-3">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <div>
                    <p className="text-xs text-foreground">Provider presets</p>
                    <p className="text-[11px] text-muted-foreground">
                      Fill endpoint defaults, then add your bucket, account, and secrets.
                    </p>
                  </div>
                </div>
                <div className="grid gap-2 md:grid-cols-2">
                  {providerPresets.map((preset) => (
                    <button
                      key={preset.id}
                      type="button"
                      className="rounded-lg border border-border bg-background px-3 py-2 text-left text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                      onClick={() => applyProviderPreset(preset)}
                    >
                      <span className="block text-foreground">{preset.label}</span>
                      <span className="mt-1 block text-[11px] leading-relaxed text-muted-foreground">
                        {preset.description}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            ) : null}

            {isOAuthStorage ? (
              <div className="rounded-xl border border-border/70 bg-card/40 p-4">
                <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                  <div className="space-y-1">
                    <p className="text-sm text-foreground">
                      {type === "google-drive" ? "Connect Google Drive" : "Connect Microsoft OneDrive"}
                    </p>
                    <p className="text-[11px] leading-relaxed text-muted-foreground">
                      Opens your browser and uses a local loopback callback with PKCE. Enter an OAuth
                      Client ID first; manual token fields below remain available as an advanced fallback.
                      Tokens stay local, and MCP exposure remains off unless you enable it.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="shrink-0 border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
                    onClick={handleOAuthConnect}
                    disabled={isOAuthConnecting}
                  >
                    <ExternalLink className="mr-2 h-4 w-4" />
                    {isOAuthConnecting ? "Waiting for browser..." : oauthConnected ? "Reconnect" : "Connect"}
                  </Button>
                </div>
                {oauthStatus ? (
                  <div className="mt-3 rounded-md border border-emerald-300/70 bg-emerald-50 px-3 py-2 text-xs text-emerald-800 dark:border-emerald-800/60 dark:bg-emerald-950/40 dark:text-emerald-200">
                    {oauthStatus}
                  </div>
                ) : null}
              </div>
            ) : null}

            <div className="grid gap-4 rounded-xl border border-border/70 bg-card/40 p-4">
              {currentSchema?.fields.map((field) => (
                <StorageFieldInput
                  key={field.name}
                  field={field}
                  value={fieldValues[field.name] ?? ""}
                  revealSecrets={revealSecrets}
                  onChange={(value) => handleFieldChange(field.name, value)}
                />
              ))}
            </div>

            {Object.keys(extraConfig).length > 0 ? (
              <p className="text-[11px] text-muted-foreground">
                This storage includes {Object.keys(extraConfig).length} advanced config field(s) not
                shown in the form. They will be preserved when you save.
              </p>
            ) : null}

            {formError ? (
              <div className="rounded-md border border-rose-300/80 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-700/60 dark:bg-rose-950/40 dark:text-rose-300">
                {formError}
              </div>
            ) : null}
          </div>

          {verifyMessage ? (
            <ValidationResultPanel
              message={verifyMessage}
              result={verifyResult}
              copyStatus={copyStatus}
              onCopy={handleCopyValidationSummary}
            />
          ) : null}

          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
              onClick={handleVerify}
              disabled={isVerifying || !onVerify}
            >
              {isVerifying ? "Validating..." : "Validate"}
            </Button>
            <Button
              type="button"
              variant="outline"
              className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              disabled={isSubmitting || !name.trim()}
            >
              {isSubmitting ? "Saving..." : isEditing ? "Save Changes" : "Add Storage"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ValidationResultPanel({
  message,
  result,
  copyStatus,
  onCopy,
}: {
  message: string;
  result: StorageValidationResult | null;
  copyStatus: string | null;
  onCopy: () => void;
}) {
  const isValid = result?.valid ?? false;
  return (
    <div
      role="status"
      aria-live="polite"
      className={`rounded-xl border px-3 py-3 text-xs ${
        isValid
          ? "border-emerald-300/80 bg-emerald-50 text-emerald-800 dark:border-emerald-700/60 dark:bg-emerald-950/40 dark:text-emerald-200"
          : "border-rose-300/80 bg-rose-50 text-rose-800 dark:border-rose-700/60 dark:bg-rose-950/40 dark:text-rose-200"
      }`}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-medium">{isValid ? "Validation passed" : "Validation needs attention"}</div>
          <div className="mt-1">{message}</div>
        </div>
        {result ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 border-current/20 bg-background/70 text-xs text-foreground hover:bg-background"
            onClick={onCopy}
            aria-label="Copy validation summary"
          >
            <Clipboard className="mr-2 h-3.5 w-3.5" />
            Copy Summary
          </Button>
        ) : null}
      </div>

      {result ? (
        <div className="mt-3 space-y-3 text-foreground">
          <div className="grid gap-2 md:grid-cols-3">
            {CAPABILITY_GROUPS.map((group) => (
              <div key={group.title} className="rounded-lg border border-current/10 bg-background/70 p-3">
                <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
                  {group.title}
                </div>
                <div className="mt-2 space-y-1.5">
                  {group.items.map((item) => (
                    <CapabilityRow
                      key={item.key}
                      label={item.label}
                      supported={Boolean(result.capabilities[item.key])}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>

          {result.fix_hints.length > 0 ? (
            <ValidationList title="Fix hints" items={result.fix_hints} />
          ) : null}
          {result.warnings.length > 0 ? (
            <ValidationList title="MCP readiness notes" items={result.warnings} />
          ) : null}
          {copyStatus ? <div className="text-[11px] text-muted-foreground">{copyStatus}</div> : null}
        </div>
      ) : null}
    </div>
  );
}

const CAPABILITY_GROUPS: Array<{
  title: string;
  items: Array<{ key: keyof StorageValidationResult["capabilities"]; label: string }>;
}> = [
  {
    title: "Browse",
    items: [
      { key: "list", label: "List" },
      { key: "stat", label: "Stat" },
      { key: "read", label: "Read" },
    ],
  },
  {
    title: "Mutate",
    items: [
      { key: "write", label: "Write" },
      { key: "delete", label: "Delete" },
      { key: "create_dir", label: "Create folder" },
      { key: "copy", label: "Copy" },
      { key: "rename", label: "Rename" },
    ],
  },
  {
    title: "Sharing & versions",
    items: [
      { key: "presign_read", label: "Download links" },
      { key: "list_with_versions", label: "List versions" },
      { key: "read_with_version", label: "Read versions" },
      { key: "delete_with_version", label: "Delete versions" },
      { key: "write_with_user_metadata", label: "User metadata" },
    ],
  },
];

function CapabilityRow({ label, supported }: { label: string; supported: boolean }) {
  return (
    <div className="flex items-center justify-between gap-2 text-[11px]">
      <span>{label}</span>
      <span
        className={`rounded-full border px-2 py-0.5 ${
          supported
            ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-200"
            : "border-muted-foreground/20 bg-muted/60 text-muted-foreground"
        }`}
      >
        {supported ? "Supported" : "Unsupported"}
      </span>
    </div>
  );
}

function ValidationList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="rounded-lg border border-current/10 bg-background/70 p-3">
      <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
        {title}
      </div>
      <ul className="mt-2 list-disc space-y-1 pl-4 text-[11px] leading-relaxed">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </div>
  );
}

function buildValidationSummary(result: StorageValidationResult): string {
  const lines = [
    `Validation: ${result.valid ? "passed" : "needs attention"}`,
    `Details: ${result.details}`,
    "Capabilities:",
    ...Object.entries(result.capabilities).map(
      ([key, value]) => `- ${key.split("_").join(" ")}: ${value ? "supported" : "unsupported"}`,
    ),
  ];
  if (result.fix_hints.length > 0) {
    lines.push("Fix hints:", ...result.fix_hints.map((hint) => `- ${hint}`));
  }
  if (result.warnings.length > 0) {
    lines.push("MCP readiness notes:", ...result.warnings.map((warning) => `- ${warning}`));
  }
  return lines.join("\n");
}

function StorageFieldInput({
  field,
  value,
  revealSecrets,
  onChange,
}: {
  field: StorageFieldSchema;
  value: string;
  revealSecrets: boolean;
  onChange: (value: string) => void;
}) {
  const isTextarea = field.input_type === "textarea";
  const isCheckbox = field.input_type === "checkbox";
  const inputType = field.secret && !revealSecrets ? "password" : field.input_type || "text";
  const inputId = `storage-field-${field.name}`;

  if (isCheckbox) {
    return (
      <div className="flex items-start justify-between gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-3">
        <Label htmlFor={inputId} className="text-xs font-normal leading-5 text-muted-foreground">
          {field.label}
          {field.required ? " *" : ""}
        </Label>
        <Switch
          id={inputId}
          checked={value === "true"}
          onCheckedChange={(checked) => onChange(String(checked))}
        />
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <Label htmlFor={inputId} className="text-xs font-normal text-muted-foreground">
        {field.label}
        {field.required ? " *" : ""}
      </Label>
      {isTextarea ? (
        <Textarea
          id={inputId}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          rows={6}
          required={field.required}
          className={`border border-border bg-[hsl(var(--card))] font-mono text-xs leading-6 text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
        />
      ) : (
        <Input
          id={inputId}
          value={value}
          type={inputType}
          required={field.required}
          onChange={(event) => onChange(event.target.value)}
          className={`border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
        />
      )}
    </div>
  );
}

function ToggleRow({
  id,
  label,
  description,
  checked,
  onCheckedChange,
}: {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-3">
      <div className="space-y-1">
        <Label htmlFor={id} className="text-sm font-medium text-foreground">
          {label}
        </Label>
        <div className="text-[11px] text-muted-foreground">{description}</div>
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function mapStorageTypeToBackend(type: StorageType): StorageDraft["backend"] {
  switch (type) {
    case "aws-s3":
      return "s3";
    case "backblaze-b2":
      return "b2";
    case "aliyun-oss":
      return "oss";
    case "tencent-cos":
      return "cos";
    case "huawei-obs":
      return "obs";
    case "azure-blob":
      return "azure_blob";
    case "webdav":
      return "webdav";
    case "gcs":
      return "gcs";
    case "google-drive":
      return "gdrive";
    case "onedrive":
      return "onedrive";
    case "sftp":
      return "sftp";
    case "ftp":
      return "ftp";
    case "local-fs":
    default:
      return "local";
  }
}

function buildFieldValues(
  schema?: StorageKindSchema,
  config: Record<string, unknown> = {},
): Record<string, string> {
  if (!schema) return {};
  return Object.fromEntries(
    schema.fields.map((field) => {
      const value = config[field.name];
      if (field.input_type === "checkbox") {
        return [field.name, stringifyCheckboxValue(value)];
      }
      return [field.name, stringifyFieldValue(value)];
    }),
  );
}

function stringifyCheckboxValue(value: unknown): string {
  if (typeof value === "boolean") return String(value);
  if (typeof value === "string") {
    return ["true", "1", "yes", "y", "on"].includes(value.trim().toLowerCase()) ? "true" : "false";
  }
  return "false";
}

function stringifyFieldValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value == null) return "";
  return JSON.stringify(value, null, 2);
}
