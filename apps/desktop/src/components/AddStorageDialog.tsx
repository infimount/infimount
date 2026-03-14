import { useEffect, useMemo, useState } from "react";
import { Eye, EyeOff, Sparkles } from "lucide-react";

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
import { listStorageSchemas, type StorageFieldSchema, type StorageKindSchema } from "@/lib/api";
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
  "azure-blob": azureIcon,
  gcs: gcsIcon,
  webdav: webdavIcon,
  "local-fs": folderNetworkIcon,
};

interface AddStorageDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd?: (config: StorageDraft) => Promise<void>;
  onUpdate?: (id: string, config: StorageDraft) => Promise<void>;
  onVerify?: (config: StorageDraft) => Promise<StorageValidationResult>;
  initialStorage?: StorageConfig;
  loadSchemas?: () => Promise<StorageKindSchema[]>;
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
}: AddStorageDialogProps) {
  const isEditing = Boolean(initialStorage);
  const [schemas, setSchemas] = useState<StorageKindSchema[]>([]);
  const [name, setName] = useState("");
  const [type, setType] = useState<StorageType>(DEFAULT_TYPE);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [extraConfig, setExtraConfig] = useState<Record<string, unknown>>({});
  const [enabled, setEnabled] = useState(true);
  const [mcpExposed, setMcpExposed] = useState(true);
  const [readOnly, setReadOnly] = useState(false);
  const [revealSecrets, setRevealSecrets] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isVerifying, setIsVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<StorageValidationResult | null>(null);
  const [verifyMessage, setVerifyMessage] = useState<string | null>(null);

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
      setRevealSecrets(!(schema?.fields.some((field) => field.secret && nextFieldValues[field.name]) ?? false));
      return;
    }

    const schema = schemas.find((item) => item.id === DEFAULT_TYPE) ?? schemas[0];
    const nextType = (schema?.id ?? DEFAULT_TYPE) as StorageType;

    setName("");
    setType(nextType);
    setFieldValues(buildFieldValues(schema));
    setExtraConfig({});
    setEnabled(true);
    setMcpExposed(true);
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

  const buildDraft = (): StorageDraft | null => {
    if (!currentSchema) {
      setFormError("Storage schema is not available yet.");
      return null;
    }

    const config: Record<string, unknown> = { ...extraConfig };

    for (const field of currentSchema.fields) {
      const rawValue = fieldValues[field.name] ?? "";
      if (field.required && !rawValue.trim()) {
        setFormError(`${field.label} is required.`);
        return null;
      }

      if (!rawValue.trim()) continue;
      config[field.name] = rawValue;
    }

    setFormError(null);
    return {
      name: name.trim(),
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

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[720px] max-h-[88vh] overflow-y-auto rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-base font-normal text-[hsl(var(--card-foreground))]">
            {isEditing ? "Edit Storage" : "Add New Storage"}
          </DialogTitle>
          <DialogDescription className="text-left text-xs text-muted-foreground">
            Configure the storage with the guided form. Full registry JSON editing is available from the storage menu.
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
                className="border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] focus-visible:border-border/60 focus-visible:ring-0 focus-visible:ring-offset-0"
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="storage-type" className="text-xs font-normal text-muted-foreground">
                Storage Type
              </Label>
              <Select value={type} onValueChange={(value) => handleTypeChange(value as StorageType)}>
                <SelectTrigger
                  id="storage-type"
                  className="border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] focus:border-border focus:ring-0 focus:ring-offset-0 focus-visible:border-border focus-visible:ring-0 focus-visible:ring-offset-0 data-[state=open]:border-border"
                >
                  <SelectValue>
                    {(() => {
                      const current = schemas.find((schema) => schema.id === type);
                      const icon = STORAGE_TYPE_ICONS[type];
                      if (!current) return null;
                      return (
                        <span className="flex items-center gap-2">
                          {icon ? <img src={icon} alt="" aria-hidden="true" className="h-4 w-4" /> : null}
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
              label="Enabled"
              description="Available in the desktop app."
              checked={enabled}
              onCheckedChange={setEnabled}
            />
            <ToggleRow
              label="Expose to MCP"
              description="Visible from the MCP virtual root."
              checked={mcpExposed}
              onCheckedChange={setMcpExposed}
            />
            <ToggleRow
              label="Read-only"
              description="Blocks writes, deletes, and moves."
              checked={readOnly}
              onCheckedChange={setReadOnly}
            />
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <Label className="text-xs font-normal text-muted-foreground">
                  Backend Fields
                </Label>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  Use the guided form for the selected backend configuration.
                </p>
              </div>
              <div className="flex items-center gap-2">
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
                This storage includes {Object.keys(extraConfig).length} advanced config field(s) not shown in the form. They will be preserved when you save.
              </p>
            ) : null}

            {formError ? (
              <div className="rounded-md border border-rose-300/80 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-700/60 dark:bg-rose-950/40 dark:text-rose-300">
                {formError}
              </div>
            ) : null}
          </div>

          {verifyMessage ? (
            <div
              className={`rounded-md border px-3 py-3 text-xs ${
                verifyResult?.valid
                  ? "border-emerald-300/80 bg-emerald-50 text-emerald-700 dark:border-emerald-700/60 dark:bg-emerald-950/40 dark:text-emerald-300"
                  : "border-rose-300/80 bg-rose-50 text-rose-700 dark:border-rose-700/60 dark:bg-rose-950/40 dark:text-rose-300"
              }`}
            >
              <div>{verifyMessage}</div>
              {verifyResult ? (
                <div className="mt-2 flex flex-wrap gap-2">
                  {Object.entries(verifyResult.capabilities)
                    .filter(([, value]) => Boolean(value))
                    .map(([key]) => (
                      <span
                        key={key}
                        className="rounded-full border border-current/20 px-2 py-1 text-[10px] uppercase tracking-[0.12em]"
                      >
                        {key.split("_").join(" ")}
                      </span>
                    ))}
                </div>
              ) : null}
            </div>
          ) : null}

          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
              onClick={handleVerify}
              disabled={isVerifying || !name.trim()}
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
  const inputType = field.secret && !revealSecrets ? "password" : field.input_type || "text";
  const inputId = `storage-field-${field.name}`;

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
          className="border border-border bg-[hsl(var(--card))] font-mono text-xs leading-6 text-[hsl(var(--card-foreground))] focus-visible:border-border/60 focus-visible:ring-0 focus-visible:ring-offset-0"
        />
      ) : (
        <Input
          id={inputId}
          value={value}
          type={inputType}
          required={field.required}
          onChange={(event) => onChange(event.target.value)}
          className="border border-border bg-[hsl(var(--card))] text-sm text-[hsl(var(--card-foreground))] focus-visible:border-border/60 focus-visible:ring-0 focus-visible:ring-offset-0"
        />
      )}
    </div>
  );
}

function ToggleRow({
  label,
  description,
  checked,
  onCheckedChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-3">
      <div className="space-y-1">
        <div className="text-sm font-medium text-foreground">{label}</div>
        <div className="text-[11px] text-muted-foreground">{description}</div>
      </div>
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function mapStorageTypeToBackend(type: StorageType): StorageDraft["backend"] {
  switch (type) {
    case "aws-s3":
      return "s3";
    case "azure-blob":
      return "azure_blob";
    case "webdav":
      return "webdav";
    case "gcs":
      return "gcs";
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
    schema.fields.map((field) => [field.name, stringifyFieldValue(config[field.name])]),
  );
}

function stringifyFieldValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value == null) return "";
  return JSON.stringify(value, null, 2);
}
