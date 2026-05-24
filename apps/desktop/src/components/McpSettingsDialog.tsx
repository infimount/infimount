import { useEffect, useState } from "react";
import {
  Bell,
  Check,
  Copy,
  Play,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Square,
  TestTube2,
  Trash2,
  X,
} from "lucide-react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
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
import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpStoragePolicy,
  McpAuditEvent,
  McpToolDefinition,
  PendingMcpConfirmation,
  StorageConfig,
} from "@/types/storage";
import type { McpNotificationPermission } from "@/lib/mcpNotifications";
import { toast } from "@/hooks/use-toast";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

const CONFIRMATION_RULES: Array<{
  key: keyof McpStoragePolicy["confirmation_rules"];
  label: string;
}> = [
  { key: "require_for_write", label: "Confirm writes" },
  { key: "require_for_overwrite", label: "Confirm overwrites" },
  { key: "require_for_delete", label: "Confirm deletes" },
  { key: "require_for_version_delete", label: "Confirm version deletes" },
  { key: "require_for_presign", label: "Confirm download links" },
  { key: "require_for_cross_storage_copy", label: "Confirm cross-storage copy" },
];

interface McpSettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: McpRuntimeStatus | null;
  snippets: McpClientSnippets | null;
  tools: McpToolDefinition[];
  storages: StorageConfig[];
  auditEvents: McpAuditEvent[];
  pendingConfirmations: PendingMcpConfirmation[];
  notificationPermission: McpNotificationPermission;
  onSave: (settings: McpSettings) => Promise<void>;
  onStartHttp: () => Promise<void>;
  onStopHttp: () => Promise<void>;
  onTestServer: () => Promise<void>;
  onRefreshAudit: () => Promise<void>;
  onClearAudit: () => Promise<void>;
  onApproveConfirmation: (operationId: string) => Promise<void>;
  onDenyConfirmation: (operationId: string) => Promise<void>;
  onEnableNotifications: () => Promise<void>;
  onUpdateStoragePolicy: (storageId: string, policy: McpStoragePolicy) => Promise<void>;
}

export function McpSettingsDialog({
  open,
  onOpenChange,
  status,
  snippets,
  tools,
  storages,
  auditEvents,
  pendingConfirmations,
  notificationPermission,
  onSave,
  onStartHttp,
  onStopHttp,
  onTestServer,
  onRefreshAudit,
  onClearAudit,
  onApproveConfirmation,
  onDenyConfirmation,
  onEnableNotifications,
  onUpdateStoragePolicy,
}: McpSettingsDialogProps) {
  const [settings, setSettings] = useState<McpSettings>({
    enabled: false,
    transport: "stdio",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: [],
  });
  const [isSaving, setIsSaving] = useState(false);
  const [isTogglingHttp, setIsTogglingHttp] = useState(false);
  const [savingPolicyId, setSavingPolicyId] = useState<string | null>(null);
  const [policyDrafts, setPolicyDrafts] = useState<Record<string, McpStoragePolicy>>({});
  const [showNetworkConfirm, setShowNetworkConfirm] = useState(false);
  const isBusy = isSaving || isTogglingHttp;
  const showNetworkWarning =
    settings.transport === "http" && !isLoopbackBindAddress(settings.bindAddress);
  const requiresHttpRestart = Boolean(
    status?.runningHttp &&
    settings.transport === "http" &&
    (settings.bindAddress !== status.settings.bindAddress ||
      settings.port !== status.settings.port ||
      !sameToolSet(settings.enabledTools, status.settings.enabledTools)),
  );

  useEffect(() => {
    if (!status || !open) return;
    setSettings(status.settings);
  }, [open, status]);

  useEffect(() => {
    if (!open) return;
    const next: Record<string, McpStoragePolicy> = {};
    for (const storage of storages) {
      next[storage.id] = clonePolicy(storage.mcpPolicy);
    }
    setPolicyDrafts(next);
  }, [open, storages]);

  const handleCopy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    toast({
      title: "Copied",
      description: "Snippet copied to clipboard.",
    });
  };

  const handleSave = async () => {
    setIsSaving(true);
    try {
      await onSave({ ...settings, enabled: false });
    } finally {
      setIsSaving(false);
    }
  };

  const handleHttpToggle = async () => {
    if (showNetworkWarning) {
      setShowNetworkConfirm(true);
      return;
    }

    await toggleHttpServer();
  };

  const updatePolicyDraft = (
    storageId: string,
    updater: (policy: McpStoragePolicy) => McpStoragePolicy,
  ) => {
    setPolicyDrafts((current) => {
      const existing =
        current[storageId] ?? storages.find((storage) => storage.id === storageId)?.mcpPolicy;
      if (!existing) return current;
      return {
        ...current,
        [storageId]: updater(clonePolicy(existing)),
      };
    });
  };

  const handleSavePolicy = async (storageId: string) => {
    const policy = policyDrafts[storageId];
    if (!policy) return;
    setSavingPolicyId(storageId);
    try {
      await onUpdateStoragePolicy(storageId, policy);
    } finally {
      setSavingPolicyId(null);
    }
  };

  const toggleHttpServer = async () => {
    setIsTogglingHttp(true);
    try {
      if (status?.runningHttp) {
        await onSave({ ...settings, enabled: false });
        await onStopHttp();
      } else {
        await onSave({ ...settings, enabled: true });
        await onStartHttp();
      }
    } finally {
      setIsTogglingHttp(false);
    }
  };

  const endpointDisplay = status?.endpointDisplay ?? "Not configured yet";
  const enabledToolCount = settings.enabledTools.length;
  const exposedStorages = storages.filter((storage) => storage.enabled && storage.mcpExposed);
  const readAccessibleStorages = exposedStorages.filter((storage) =>
    storageAllowsRead(storage, policyDrafts[storage.id]),
  );
  const writeAccessibleStorages = exposedStorages.filter((storage) =>
    storageAllowsWrite(storage, policyDrafts[storage.id]),
  );
  const writeToolsEnabled = settings.enabledTools.some((name) =>
    ["write_file", "mkdir", "copy_path", "move_path"].includes(name),
  );
  const destructiveToolsEnabled = settings.enabledTools.some((name) =>
    ["delete_path", "delete_version", "move_path"].includes(name),
  );
  const presignEnabled = settings.enabledTools.includes("generate_download_link");
  const writeAccessSummary = writeToolsEnabled
    ? writeAccessibleStorages.length > 0
      ? `Enabled for ${formatStorageCount(writeAccessibleStorages.length)}`
      : "Blocked by read-only or no-access policies"
    : "No write tools enabled";
  const destructiveAccessSummary = destructiveToolsEnabled
    ? writeAccessibleStorages.length > 0
      ? `Enabled for ${formatStorageCount(writeAccessibleStorages.length)}`
      : "Blocked by read-only or no-access policies"
    : "No destructive tools enabled";
  const readAccessSummary =
    readAccessibleStorages.length > 0
      ? `Allowed for ${formatStorageCount(readAccessibleStorages.length)}`
      : "No exposed policy allows reads";
  const presignSummary = presignEnabled
    ? readAccessibleStorages.length > 0
      ? `Enabled for ${formatStorageCount(readAccessibleStorages.length)}`
      : "Blocked by no-access policies"
    : "Disabled";
  const primaryActionLabel =
    settings.transport === "http"
      ? status?.runningHttp
        ? "Stop HTTP Server"
        : "Save & Start HTTP Server"
      : "Save MCP Settings";

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="sm:max-w-[760px] max-h-[88vh] overflow-y-auto rounded-2xl border border-border bg-background text-foreground shadow-2xl">
          <DialogHeader>
            <DialogTitle className="text-left text-base font-normal text-[hsl(var(--card-foreground))]">
              MCP Settings
            </DialogTitle>
            <DialogDescription className="text-left text-xs text-muted-foreground">
              Configure the MCP runtime that Infimount exposes locally for external clients.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-5">
            <div className="grid gap-4 rounded-xl border border-border/80 bg-secondary/35 p-4 md:grid-cols-[1.1fr_0.9fr]">
              <div className="space-y-4">
                <div>
                  <Label className="text-sm font-medium text-foreground">Transport Settings</Label>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Choose how clients connect. HTTP settings are applied when you start the server.
                  </p>
                </div>

                <div className="space-y-2">
                  <Label className="text-xs font-normal text-muted-foreground">Transport</Label>
                  <Select
                    value={settings.transport}
                    onValueChange={(value) =>
                      setSettings((current) => ({
                        ...current,
                        transport: value as McpSettings["transport"],
                      }))
                    }
                  >
                    <SelectTrigger
                      className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                      <SelectItem value="stdio">stdio</SelectItem>
                      <SelectItem value="http">http</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {settings.transport === "http" ? (
                  <div className="space-y-3">
                    <div className="grid gap-4 md:grid-cols-2">
                      <div className="space-y-2">
                        <Label className="text-xs font-normal text-muted-foreground">
                          Bind Address
                        </Label>
                        <Input
                          value={settings.bindAddress}
                          onChange={(event) =>
                            setSettings((current) => ({
                              ...current,
                              bindAddress: event.target.value,
                            }))
                          }
                          className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                        />
                      </div>
                      <div className="space-y-2">
                        <Label className="text-xs font-normal text-muted-foreground">Port</Label>
                        <Input
                          type="number"
                          min={0}
                          max={65535}
                          value={settings.port}
                          onChange={(event) =>
                            setSettings((current) => ({
                              ...current,
                              port: Number.parseInt(event.target.value || "0", 10),
                            }))
                          }
                          className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                        />
                      </div>
                    </div>
                    {showNetworkWarning ? (
                      <div className="rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                        This bind address is not loopback. Clients on your LAN may be able to reach
                        this MCP endpoint.
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>

              <div className="space-y-3 rounded-lg border border-border/80 bg-card p-4 shadow-sm">
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <div className="text-sm font-medium text-foreground">Runtime Status</div>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {status?.runningHttp ? "HTTP server is live." : "HTTP server is not running."}
                    </p>
                  </div>
                  <div
                    className={`rounded-full px-2.5 py-1 text-[10px] font-medium uppercase tracking-[0.14em] ${
                      status?.runningHttp
                        ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                        : "bg-muted text-muted-foreground"
                    }`}
                  >
                    {status?.runningHttp ? "Running" : "Stopped"}
                  </div>
                </div>

                <div className="rounded-lg border border-border/80 bg-background p-3">
                  <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
                    Endpoint
                  </div>
                  <div className="mt-2 break-all font-mono text-xs text-foreground">
                    {endpointDisplay}
                  </div>
                </div>

                {settings.transport === "http" ? (
                  <Button
                    type="button"
                    className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
                    onClick={handleHttpToggle}
                    disabled={isBusy}
                  >
                    {status?.runningHttp ? (
                      <Square className="mr-2 h-4 w-4" />
                    ) : (
                      <Play className="mr-2 h-4 w-4" />
                    )}
                    {isTogglingHttp ? "Working..." : primaryActionLabel}
                  </Button>
                ) : (
                  <Button
                    type="button"
                    className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
                    onClick={handleSave}
                    disabled={isBusy}
                  >
                    {isSaving ? "Saving..." : primaryActionLabel}
                  </Button>
                )}

                {requiresHttpRestart ? (
                  <div className="rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                    MCP settings changed. Restart the HTTP server (Stop then Start) to apply these
                    changes.
                  </div>
                ) : null}
              </div>
            </div>

            <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <div className="text-sm font-medium text-foreground">Exposed MCP Functions</div>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Enable only the MCP tools you want to expose. Disabled tools are hidden and
                    cannot be called.
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="h-8 border-border/80"
                    onClick={() =>
                      setSettings((current) => ({
                        ...current,
                        enabledTools: tools.map((tool) => tool.name),
                      }))
                    }
                    disabled={isBusy || tools.length === 0}
                  >
                    Enable all
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="h-8 border-border/80"
                    onClick={() => setSettings((current) => ({ ...current, enabledTools: [] }))}
                    disabled={isBusy || tools.length === 0}
                  >
                    Disable all
                  </Button>
                </div>
              </div>

              <div className="text-xs text-muted-foreground">
                {enabledToolCount} of {tools.length} functions enabled
              </div>

              <div className="max-h-60 space-y-2 overflow-y-auto rounded-lg border border-border/80 bg-background p-2">
                {tools.map((tool) => {
                  const checked = settings.enabledTools.includes(tool.name);
                  return (
                    <div
                      key={tool.name}
                      className="flex items-start justify-between gap-4 rounded-md border border-transparent px-2 py-2 hover:bg-secondary/50"
                    >
                      <div className="space-y-1">
                        <div className="font-mono text-xs text-foreground">{tool.name}</div>
                        <p className="text-xs text-muted-foreground">{tool.description}</p>
                      </div>
                      <Switch
                        checked={checked}
                        disabled={isBusy}
                        onCheckedChange={(value) =>
                          setSettings((current) => ({
                            ...current,
                            enabledTools: value
                              ? [...current.enabledTools, tool.name].sort()
                              : current.enabledTools.filter((name) => name !== tool.name),
                          }))
                        }
                      />
                    </div>
                  );
                })}
                {tools.length === 0 ? (
                  <div className="px-2 py-3 text-xs text-muted-foreground">
                    Tool metadata is unavailable.
                  </div>
                ) : null}
              </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <SnippetCard
                title="Stdio Snippet"
                value={snippets?.stdio ?? ""}
                onCopy={() => handleCopy(snippets?.stdio ?? "")}
              />
              <SnippetCard
                title="HTTP Snippet"
                value={snippets?.http ?? ""}
                onCopy={() => handleCopy(snippets?.http ?? "")}
              />
            </div>

            <div className="grid gap-4 md:grid-cols-[0.9fr_1.1fr]">
              <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
                <div className="flex items-start gap-3">
                  <ShieldCheck className="mt-0.5 h-4 w-4 text-primary" />
                  <div>
                    <div className="text-sm font-medium text-foreground">
                      What the agent can access
                    </div>
                    <p className="mt-1 text-xs leading-5 text-muted-foreground">
                      This summary reflects enabled storages and currently enabled MCP functions.
                    </p>
                  </div>
                </div>
                <div className="space-y-2 text-xs">
                  <SummaryRow label="Exposed storages" value={`${exposedStorages.length}`} />
                  <SummaryRow label="Enabled functions" value={`${enabledToolCount}`} />
                  <SummaryRow label="Read access" value={readAccessSummary} />
                  <SummaryRow label="Write access" value={writeAccessSummary} />
                  <SummaryRow label="Destructive access" value={destructiveAccessSummary} />
                  <SummaryRow label="Presigned links" value={presignSummary} />
                </div>
              </div>

              <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <div className="text-sm font-medium text-foreground">MCP Setup Wizard</div>
                    <p className="mt-1 text-xs leading-5 text-muted-foreground">
                      Confirm runtime status, enabled functions, and exposed storage count before
                      connecting an agent.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="border-border/80"
                    onClick={() => void onTestServer()}
                    disabled={isBusy}
                  >
                    <TestTube2 className="mr-2 h-4 w-4" />
                    Test
                  </Button>
                </div>
                <div className="rounded-lg border border-border/80 bg-background p-3">
                  <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
                    Exposed Storages
                  </div>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {exposedStorages.length > 0 ? (
                      exposedStorages.map((storage) => (
                        <span
                          key={storage.id}
                          className="rounded-full border border-border bg-secondary/50 px-2 py-1 text-xs text-foreground"
                        >
                          {storage.name}
                        </span>
                      ))
                    ) : (
                      <span className="text-xs text-muted-foreground">
                        No storage is currently exposed to MCP.
                      </span>
                    )}
                  </div>
                </div>
              </div>
            </div>

            <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
              <div>
                <div className="text-sm font-medium text-foreground">Path Policies</div>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  Restrict exposed storages by access mode and path prefixes before any MCP tool
                  reaches the backend.
                </p>
              </div>

              <div className="space-y-3">
                {exposedStorages.length > 0 ? (
                  exposedStorages.map((storage) => {
                    const policy = policyDrafts[storage.id] ?? storage.mcpPolicy;
                    return (
                      <div
                        key={storage.id}
                        className="space-y-3 rounded-lg border border-border/80 bg-background p-3"
                      >
                        <div className="flex flex-wrap items-center justify-between gap-3">
                          <div>
                            <div className="text-sm font-medium text-foreground">
                              {storage.name}
                            </div>
                            <div className="text-xs text-muted-foreground">{storage.backend}</div>
                          </div>
                          <div className="w-44">
                            <Select
                              value={policy.default_access}
                              onValueChange={(value) =>
                                updatePolicyDraft(storage.id, (current) => ({
                                  ...current,
                                  default_access: value as McpStoragePolicy["default_access"],
                                }))
                              }
                            >
                              <SelectTrigger
                                className={`h-9 border border-border bg-card text-xs text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                              >
                                <SelectValue />
                              </SelectTrigger>
                              <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                                <SelectItem value="none">No access</SelectItem>
                                <SelectItem value="read_only">Read only</SelectItem>
                                <SelectItem value="read_write">Read / write</SelectItem>
                              </SelectContent>
                            </Select>
                          </div>
                        </div>

                        <div className="grid gap-3 md:grid-cols-2">
                          <div className="space-y-2">
                            <Label className="text-xs font-normal text-muted-foreground">
                              Allowed prefixes
                            </Label>
                            <Textarea
                              value={policy.allowed_paths.join("\n")}
                              placeholder="Leave empty to allow all paths"
                              rows={3}
                              className={`border border-border/80 bg-card font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                              onChange={(event) =>
                                updatePolicyDraft(storage.id, (current) => ({
                                  ...current,
                                  allowed_paths: splitPolicyPrefixes(event.target.value),
                                }))
                              }
                            />
                          </div>
                          <div className="space-y-2">
                            <Label className="text-xs font-normal text-muted-foreground">
                              Denied prefixes
                            </Label>
                            <Textarea
                              value={policy.denied_paths.join("\n")}
                              placeholder="Example: private"
                              rows={3}
                              className={`border border-destructive/30 bg-destructive/5 font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                              onChange={(event) =>
                                updatePolicyDraft(storage.id, (current) => ({
                                  ...current,
                                  denied_paths: splitPolicyPrefixes(event.target.value),
                                }))
                              }
                            />
                          </div>
                        </div>

                        <div className="grid gap-2 md:grid-cols-2">
                          {CONFIRMATION_RULES.map((rule) => (
                            <div
                              key={rule.key}
                              className="flex items-center justify-between gap-3 rounded-md border border-border/70 bg-card px-3 py-2"
                            >
                              <span className="text-xs text-muted-foreground">{rule.label}</span>
                              <Switch
                                checked={policy.confirmation_rules[rule.key]}
                                onCheckedChange={(value) =>
                                  updatePolicyDraft(storage.id, (current) => ({
                                    ...current,
                                    confirmation_rules: {
                                      ...current.confirmation_rules,
                                      [rule.key]: value,
                                    },
                                  }))
                                }
                              />
                            </div>
                          ))}
                        </div>

                        <div className="flex justify-end">
                          <Button
                            type="button"
                            size="sm"
                            className="h-8 bg-primary text-primary-foreground hover:bg-primary/90"
                            onClick={() => void handleSavePolicy(storage.id)}
                            disabled={savingPolicyId === storage.id}
                          >
                            {savingPolicyId === storage.id ? "Saving..." : "Save policy"}
                          </Button>
                        </div>
                      </div>
                    );
                  })
                ) : (
                  <div className="rounded-lg border border-border/80 bg-background px-3 py-6 text-center text-xs text-muted-foreground">
                    No storage is exposed to MCP yet.
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex items-start gap-3">
                  <ShieldAlert className="mt-0.5 h-4 w-4 text-primary" />
                  <div>
                    <div className="text-sm font-medium text-foreground">
                      Pending MCP Approvals
                    </div>
                    <p className="mt-1 text-xs leading-5 text-muted-foreground">
                      Risky agent operations wait here. Notifications can point back to this queue,
                      but approval happens in Infimount.
                    </p>
                  </div>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  {notificationPermission !== "granted" &&
                    notificationPermission !== "unsupported" && (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="border-border/80"
                        onClick={() => void onEnableNotifications()}
                      >
                        <Bell className="mr-2 h-4 w-4" />
                        Enable notifications
                      </Button>
                    )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="border-border/80"
                    onClick={() => void onRefreshAudit()}
                  >
                    <RefreshCw className="mr-2 h-4 w-4" />
                    Refresh
                  </Button>
                </div>
              </div>

              <div className="max-h-64 overflow-y-auto rounded-lg border border-border/80 bg-background">
                {pendingConfirmations.length > 0 ? (
                  pendingConfirmations.map((item) => (
                    <div
                      key={item.operation_id}
                      className="grid gap-3 border-b border-border/70 px-3 py-3 text-xs last:border-b-0 md:grid-cols-[1fr_auto]"
                    >
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-mono text-foreground">{item.tool_name}</span>
                          <span className="rounded-full border border-amber-300/80 bg-amber-50 px-2 py-0.5 text-[11px] text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                            {item.risk_type}
                          </span>
                          <span className="rounded-full border border-border bg-secondary/60 px-2 py-0.5 text-[11px] text-muted-foreground">
                            {item.operation}
                          </span>
                        </div>
                        <div className="truncate text-muted-foreground">{item.summary}</div>
                        <div className="truncate text-muted-foreground">
                          {item.storage_name} · {item.path}
                        </div>
                        <div className="text-[11px] text-muted-foreground">
                          Expires {new Date(item.expires_at).toLocaleString()}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 md:justify-end">
                        <Button
                          type="button"
                          size="sm"
                          className="h-8 bg-primary text-primary-foreground hover:bg-primary/90"
                          onClick={() => void onApproveConfirmation(item.operation_id)}
                        >
                          <Check className="mr-2 h-4 w-4" />
                          Approve once
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="h-8 border-border/80"
                          onClick={() => void onDenyConfirmation(item.operation_id)}
                        >
                          <X className="mr-2 h-4 w-4" />
                          Deny
                        </Button>
                      </div>
                    </div>
                  ))
                ) : (
                  <div className="px-3 py-6 text-center text-xs text-muted-foreground">
                    No risky MCP operations are waiting for approval.
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <div className="text-sm font-medium text-foreground">MCP Audit</div>
                  <p className="mt-1 text-xs leading-5 text-muted-foreground">
                    Recent local MCP calls, denials, and failures. Secrets and presigned URL
                    signatures are not stored here.
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="border-border/80"
                    onClick={() => void onRefreshAudit()}
                  >
                    <RefreshCw className="mr-2 h-4 w-4" />
                    Refresh
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="border-border/80"
                    onClick={() => void onClearAudit()}
                    disabled={auditEvents.length === 0}
                  >
                    <Trash2 className="mr-2 h-4 w-4" />
                    Clear
                  </Button>
                </div>
              </div>
              <div className="max-h-64 overflow-y-auto rounded-lg border border-border/80 bg-background">
                {auditEvents.length > 0 ? (
                  auditEvents.slice(0, 20).map((event) => (
                    <div
                      key={event.id}
                      className="grid gap-1 border-b border-border/70 px-3 py-2 text-xs last:border-b-0 md:grid-cols-[1fr_auto]"
                    >
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-mono text-foreground">{event.tool_name}</span>
                          <span className="rounded-full border border-border bg-secondary/50 px-2 py-0.5 text-[11px] text-muted-foreground">
                            {event.decision}
                          </span>
                          {event.error_code ? (
                            <span className="text-[11px] text-destructive">
                              {event.error_code}
                            </span>
                          ) : null}
                        </div>
                        <div className="mt-1 truncate text-muted-foreground">
                          {event.storage_name ?? "-"} {event.path ? `· ${event.path}` : ""}
                        </div>
                      </div>
                      <div className="text-muted-foreground md:text-right">
                        {new Date(event.timestamp).toLocaleString()}
                      </div>
                    </div>
                  ))
                ) : (
                  <div className="px-3 py-6 text-center text-xs text-muted-foreground">
                    No MCP audit events yet.
                  </div>
                )}
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog open={showNetworkConfirm} onOpenChange={setShowNetworkConfirm}>
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Expose MCP beyond this machine?</AlertDialogTitle>
            <AlertDialogDescription>
              The HTTP server will bind to {settings.bindAddress}. Clients on your LAN may be able
              to reach this endpoint. Continue only if you intend to expose it.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={() => {
                setShowNetworkConfirm(false);
                void toggleHttpServer();
              }}
            >
              Start Server
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function isLoopbackBindAddress(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return (
    normalized === "127.0.0.1" ||
    normalized === "localhost" ||
    normalized === "::1" ||
    normalized === "[::1]"
  );
}

function sameToolSet(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  const aSorted = [...a].sort();
  const bSorted = [...b].sort();
  return aSorted.every((name, index) => name === bSorted[index]);
}

function clonePolicy(policy: McpStoragePolicy): McpStoragePolicy {
  return {
    default_access: policy.default_access,
    allowed_paths: [...policy.allowed_paths],
    denied_paths: [...policy.denied_paths],
    confirmation_rules: { ...policy.confirmation_rules },
  };
}

function effectivePolicy(storage: StorageConfig, draft?: McpStoragePolicy): McpStoragePolicy {
  return draft ?? storage.mcpPolicy;
}

function storageAllowsRead(storage: StorageConfig, draft?: McpStoragePolicy): boolean {
  const policy = effectivePolicy(storage, draft);
  return policy.default_access === "read_only" || policy.default_access === "read_write";
}

function storageAllowsWrite(storage: StorageConfig, draft?: McpStoragePolicy): boolean {
  const policy = effectivePolicy(storage, draft);
  return !storage.readOnly && policy.default_access === "read_write";
}

function formatStorageCount(count: number): string {
  return `${count} ${count === 1 ? "storage" : "storages"}`;
}

function splitPolicyPrefixes(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/\r?\n/)
    .map(normalizePolicyPrefixInput)
    .filter(Boolean)
    .filter((item) => {
      if (seen.has(item)) {
        return false;
      }
      seen.add(item);
      return true;
    });
}

function normalizePolicyPrefixInput(value: string): string {
  const segments: string[] = [];
  for (const segment of value.trim().replace(/\\/g, "/").split("/")) {
    if (!segment || segment === ".") {
      continue;
    }
    if (segment === "..") {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.join("/");
}

function SummaryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background px-3 py-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-right text-foreground">{value}</span>
    </div>
  );
}

function SnippetCard({
  title,
  value,
  onCopy,
}: {
  title: string;
  value: string;
  onCopy: () => void;
}) {
  return (
    <div className="space-y-2 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-medium text-foreground">{title}</div>
        <Button
          type="button"
          variant="outline"
          className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
          onClick={onCopy}
          disabled={!value}
        >
          <Copy className="mr-2 h-4 w-4" />
          Copy
        </Button>
      </div>
      <Textarea
        value={value}
        readOnly
        rows={12}
        className={`min-h-[220px] border border-border/80 bg-background font-mono text-xs leading-6 text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
      />
    </div>
  );
}
