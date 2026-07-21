import { useEffect, useState } from "react";

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
import type {
  ActiveMcpSession,
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpSettingsUpdate,
  McpConfirmationRules,
  McpStoragePolicy,
  McpAuditEvent,
  McpToolDefinition,
  PendingMcpConfirmation,
  StorageConfig,
} from "@/types/storage";
import type { McpNotificationPermission } from "@/lib/mcpNotifications";
import { toast } from "@/hooks/use-toast";

import { McpRuntimeSection } from "./McpRuntimeSection";
import { McpToolSection } from "./McpToolSection";
import { McpPolicySection } from "./McpPolicySection";
import { McpAuditSection } from "./McpAuditSection";

interface McpSettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: McpRuntimeStatus | null;
  snippets: McpClientSnippets | null;
  tools: McpToolDefinition[];
  storages: StorageConfig[];
  auditEvents: McpAuditEvent[];
  pendingConfirmations: PendingMcpConfirmation[];
  activeSessions: ActiveMcpSession[];
  notificationPermission: McpNotificationPermission;
  onSave: (settings: McpSettingsUpdate) => Promise<void>;
  onStartHttp: () => Promise<void>;
  onStopHttp: () => Promise<void>;
  onTestServer: () => Promise<void>;
  onRefreshAudit: () => Promise<void>;
  onClearAudit: () => Promise<void>;
  onExportAuditBundle: (events: McpAuditEvent[]) => Promise<void>;
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
  activeSessions,
  notificationPermission,
  onSave,
  onStartHttp,
  onStopHttp,
  onTestServer,
  onRefreshAudit,
  onClearAudit,
  onExportAuditBundle,
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
    securityBaselineVersion: 2,
    authTokenConfigured: false,
  });
  const [authTokenDraft, setAuthTokenDraft] = useState<string | undefined>();
  const [isSaving, setIsSaving] = useState(false);
  const [isTogglingHttp, setIsTogglingHttp] = useState(false);
  const [savingPolicyId, setSavingPolicyId] = useState<string | null>(null);
  const [applyingPresetId, setApplyingPresetId] = useState<string | null>(null);
  const [policyDrafts, setPolicyDrafts] = useState<Record<string, McpStoragePolicy>>({});
  const [auditQuery, setAuditQuery] = useState("");
  const [auditDecisionFilter, setAuditDecisionFilter] = useState("all");
  const [auditStorageFilter, setAuditStorageFilter] = useState("all");
  const [showNetworkConfirm, setShowNetworkConfirm] = useState(false);
  const isBusy = isSaving || isTogglingHttp || applyingPresetId !== null;
  const showNetworkWarning =
    settings.transport === "http" && !isLoopbackBindAddress(settings.bindAddress);
  const httpAuthToken = authTokenDraft?.trim() ?? "";
  const effectiveAuthConfigured =
    authTokenDraft === undefined ? settings.authTokenConfigured : httpAuthToken.length > 0;
  const nonLoopbackMissingAuth = showNetworkWarning && !effectiveAuthConfigured;
  const requiresHttpRestart = Boolean(
    status?.runningHttp &&
    settings.transport === "http" &&
    (settings.bindAddress !== status.settings.bindAddress ||
      settings.port !== status.settings.port ||
      authTokenDraft !== undefined ||
      !sameToolSet(settings.enabledTools, status.settings.enabledTools)),
  );

  useEffect(() => {
    if (!status || !open) return;
    setSettings(status.settings);
    setAuthTokenDraft(undefined);
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
      await onSave(toSettingsUpdate(settings, authTokenDraft, false));
      setAuthTokenDraft(undefined);
    } finally {
      setIsSaving(false);
    }
  };

  const handleHttpToggle = async () => {
    if (!status?.runningHttp && showNetworkWarning) {
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

    // Reject blank rules (empty prefix) and root grants
    for (const rule of policy.rules) {
      const trimmed = rule.prefix.trim();
      if (!trimmed) {
        toast({
          title: "Cannot save policy",
          description: "A rule has an empty path prefix. Remove it or enter a prefix.",
          variant: "destructive",
        });
        return;
      }
      if (trimmed === "/" || trimmed === "." || trimmed === "..") {
        toast({
          title: "Cannot save policy",
          description: `Rule "${rule.id}" has an invalid root prefix '${trimmed}'. Root grants are not allowed.`,
          variant: "destructive",
        });
        return;
      }
    }

    setSavingPolicyId(storageId);
    try {
      await onUpdateStoragePolicy(storageId, policy);
    } finally {
      setSavingPolicyId(null);
    }
  };

  const handleApplyPreset = async (preset: { id: string; title: string; enabledTools: string[]; accessMode: McpStoragePolicy["default_access"]; confirmationRules: McpConfirmationRules }) => {
    const nextSettings: McpSettings = {
      ...settings,
      enabled: false,
      enabledTools: filterAvailableTools(preset.enabledTools, tools),
    };
    const nextDrafts = buildPresetPolicyDrafts(exposedStorages, policyDrafts, preset);

    setApplyingPresetId(preset.id);

    // Apply policy updates BEFORE tool settings; rollback on failure
    const savedDrafts: Record<string, McpStoragePolicy> = {};
    try {
      for (const storage of exposedStorages) {
        savedDrafts[storage.id] = clonePolicy(effectivePolicy(storage, policyDrafts[storage.id]));
        await onUpdateStoragePolicy(storage.id, nextDrafts[storage.id]);
      }
    } catch {
      // Rollback policy changes
      for (const storage of exposedStorages) {
        if (savedDrafts[storage.id]) {
          await onUpdateStoragePolicy(storage.id, savedDrafts[storage.id]).catch(() => {});
        }
      }
      setApplyingPresetId(null);
      toast({
        title: "Preset failed",
        description: "Policy updates could not be saved. Changes rolled back.",
        variant: "destructive",
      });
      return;
    }

    // Apply tool settings only after policy updates succeed
    try {
      await onSave(toSettingsUpdate(nextSettings, undefined, nextSettings.enabled));
      setPolicyDrafts((current) => ({ ...current, ...nextDrafts }));
      toast({
        title: "Preset applied",
        description:
          status?.runningHttp && settings.transport === "http"
            ? "Restart the HTTP server to apply tool changes to connected clients."
            : `${preset.title} is saved for exposed MCP storage.`,
      });
    } catch {
      // Rollback tool settings to previous state
      for (const storage of exposedStorages) {
        if (savedDrafts[storage.id]) {
          await onUpdateStoragePolicy(storage.id, savedDrafts[storage.id]).catch(() => {});
        }
      }
      toast({
        title: "Preset failed",
        description: "Tool settings could not be saved. Policy changes rolled back.",
        variant: "destructive",
      });
    } finally {
      setApplyingPresetId(null);
    }
  };

  const handleCopyVisibleAudit = async () => {
    await navigator.clipboard.writeText(JSON.stringify(filteredAuditEvents, null, 2));
    toast({
      title: "Audit copied",
      description: `${filteredAuditEvents.length} visible MCP audit events copied as JSON.`,
    });
  };

  const handleExportVisibleAudit = async () => {
    await onExportAuditBundle(filteredAuditEvents);
  };

  const toggleHttpServer = async () => {
    setIsTogglingHttp(true);
    try {
      if (status?.runningHttp) {
        await onSave(toSettingsUpdate(settings, authTokenDraft, false));
        setAuthTokenDraft(undefined);
        await onStopHttp();
      } else {
        await onSave(toSettingsUpdate(settings, authTokenDraft, true));
        setAuthTokenDraft(undefined);
        await onStartHttp();
      }
    } finally {
      setIsTogglingHttp(false);
    }
  };

  const endpointDisplay = status?.endpointDisplay ?? "Not configured yet";
  const enabledToolCount = settings.enabledTools.length;
  const exposedStorages = storages.filter((storage) => storage.enabled && storage.mcpExposed);
  const accessCounts = summarizeStorageAccess(exposedStorages, policyDrafts);
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
  const confirmationSummary = summarizeConfirmationRules(exposedStorages, policyDrafts);
  const connectAssessment = assessMcpConnectionSafety({
    exposedStorageCount: exposedStorages.length,
    enabledToolCount,
    showNetworkWarning,
    destructiveAccessEnabled: destructiveToolsEnabled && writeAccessibleStorages.length > 0,
  });
  const auditStorageOptions = Array.from(
    new Set(auditEvents.map((event) => event.storage_name).filter((name): name is string => Boolean(name))),
  ).sort();
  const auditDecisionOptions = Array.from(new Set(auditEvents.map((event) => event.decision))).sort();
  const filteredAuditEvents = filterAuditEvents(auditEvents, {
    query: auditQuery,
    decision: auditDecisionFilter,
    storage: auditStorageFilter,
  });
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
            <McpRuntimeSection
              settings={settings}
              onSettingsChange={setSettings}
              status={status}
              authTokenDraft={authTokenDraft}
              onAuthTokenDraftChange={setAuthTokenDraft}
              isBusy={isBusy}
              isSaving={isSaving}
              isTogglingHttp={isTogglingHttp}
              nonLoopbackMissingAuth={nonLoopbackMissingAuth}
              showNetworkWarning={showNetworkWarning}
              requiresHttpRestart={requiresHttpRestart}
              primaryActionLabel={primaryActionLabel}
              endpointDisplay={endpointDisplay}
              onSave={handleSave}
              onHttpToggle={handleHttpToggle}
            />

            <McpToolSection
              tools={tools}
              settings={settings}
              onSettingsChange={setSettings}
              isBusy={isBusy}
              snippets={snippets}
              onCopy={handleCopy}
              exposedStorages={exposedStorages}
              policyDrafts={policyDrafts}
              onApplyPreset={handleApplyPreset}
              applyingPresetId={applyingPresetId}
              connectAssessment={connectAssessment}
              accessCounts={accessCounts}
              readAccessSummary={readAccessSummary}
              writeAccessSummary={writeAccessSummary}
              destructiveAccessSummary={destructiveAccessSummary}
              presignSummary={presignSummary}
              confirmationSummary={confirmationSummary}
              showNetworkWarning={showNetworkWarning}
              activeSessions={activeSessions}
              onTestServer={onTestServer}
            />

            <McpPolicySection
              exposedStorages={exposedStorages}
              policyDrafts={policyDrafts}
              onUpdatePolicyDraft={updatePolicyDraft}
              onSavePolicy={handleSavePolicy}
              savingPolicyId={savingPolicyId}
            />

            <McpAuditSection
              activeSessions={activeSessions}
              pendingConfirmations={pendingConfirmations}
              auditEvents={auditEvents}
              notificationPermission={notificationPermission}
              onRefreshAudit={onRefreshAudit}
              onClearAudit={onClearAudit}
              onEnableNotifications={onEnableNotifications}
              onApproveConfirmation={onApproveConfirmation}
              onDenyConfirmation={onDenyConfirmation}
              onCopyVisibleAudit={handleCopyVisibleAudit}
              onExportVisibleAudit={handleExportVisibleAudit}
              filteredAuditEvents={filteredAuditEvents}
              auditQuery={auditQuery}
              onAuditQueryChange={setAuditQuery}
              auditDecisionFilter={auditDecisionFilter}
              onAuditDecisionFilterChange={setAuditDecisionFilter}
              auditStorageFilter={auditStorageFilter}
              onAuditStorageFilterChange={setAuditStorageFilter}
              auditDecisionOptions={auditDecisionOptions}
              auditStorageOptions={auditStorageOptions}
            />
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog open={showNetworkConfirm} onOpenChange={setShowNetworkConfirm}>
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Expose MCP beyond this machine?</AlertDialogTitle>
            <AlertDialogDescription>
              The HTTP server will bind to {settings.bindAddress}. Clients on your LAN may be able
              to reach it. Continue only if this is intentional, a bearer token is configured, and
              your storage exposure policies are scoped for this network.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              disabled={nonLoopbackMissingAuth}
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

function toSettingsUpdate(
  settings: McpSettings,
  authTokenDraft: string | undefined,
  enabled: boolean,
): McpSettingsUpdate {
  const token = authTokenDraft?.trim();
  return {
    enabled,
    transport: settings.transport,
    bindAddress: settings.bindAddress,
    port: settings.port,
    enabledTools: settings.enabledTools,
    authTokenMutation:
      authTokenDraft === undefined
        ? { action: "keep" }
        : token
          ? { action: "set", value: token }
          : { action: "clear" },
  };
}

function clonePolicy(policy: McpStoragePolicy): McpStoragePolicy {
  return {
    version: 2,
    default_access: policy.default_access,
    rules: [...policy.rules],
    allowed_paths: [...(policy.allowed_paths ?? [])],
    denied_paths: [...policy.denied_paths],
    confirmation_rules: { ...policy.confirmation_rules },
  };
}

function filterAvailableTools(toolNames: string[], tools: McpToolDefinition[]): string[] {
  const available = new Set(tools.map((tool) => tool.name));
  return toolNames.filter((name) => available.has(name)).sort();
}

function buildPresetPolicyDrafts(
  storages: StorageConfig[],
  drafts: Record<string, McpStoragePolicy>,
  preset: { id: string; accessMode: McpStoragePolicy["default_access"]; confirmationRules: McpConfirmationRules },
): Record<string, McpStoragePolicy> {
  const next: Record<string, McpStoragePolicy> = {};
  for (const storage of storages) {
    const current = clonePolicy(effectivePolicy(storage, drafts[storage.id]));

    if (preset.id === "locked-down") {
      next[storage.id] = {
        ...current,
        default_access: "none",
        confirmation_rules: { ...preset.confirmationRules },
      };
      continue;
    }

    if (preset.id === "research-read-only") {
      next[storage.id] = {
        ...current,
        default_access:
          current.default_access === "none" ? "none" : "read_only",
        rules: current.rules.map((rule) => ({
          ...rule,
          access: rule.access === "none" ? "none" : "read_only",
        })),
        confirmation_rules: { ...preset.confirmationRules },
      };
      continue;
    }

    // Workspace Agent and Manual Approval change tool availability only. Existing
    // whole-storage defaults and path grants remain exactly as selected by users.
    next[storage.id] = {
      ...current,
      confirmation_rules: { ...preset.confirmationRules },
    };
  }
  return next;
}

function filterAuditEvents(
  events: McpAuditEvent[],
  filters: { query: string; decision: string; storage: string },
): McpAuditEvent[] {
  const query = filters.query.trim().toLowerCase();
  return events.filter((event) => {
    if (filters.decision !== "all" && event.decision !== filters.decision) return false;
    if (filters.storage !== "all" && event.storage_name !== filters.storage) return false;
    if (!query) return true;

    return [
      event.tool_name,
      event.operation,
      event.path,
      event.storage_name,
      event.backend,
      event.session_id,
      event.error_code,
      event.decision,
      event.matched_rule_id,
      event.workspace_id,
    ]
      .filter(Boolean)
      .some((value) => value?.toLowerCase().includes(query));
  });
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

function summarizeStorageAccess(
  storages: StorageConfig[],
  drafts: Record<string, McpStoragePolicy>,
): { noAccess: number; readOnly: number; readWrite: number } {
  return storages.reduce(
    (counts, storage) => {
      const policy = effectivePolicy(storage, drafts[storage.id]);
      if (policy.default_access === "none") {
        counts.noAccess += 1;
      } else if (storage.readOnly || policy.default_access === "read_only") {
        counts.readOnly += 1;
      } else {
        counts.readWrite += 1;
      }
      return counts;
    },
    { noAccess: 0, readOnly: 0, readWrite: 0 },
  );
}

function summarizeConfirmationRules(
  storages: StorageConfig[],
  drafts: Record<string, McpStoragePolicy>,
): string {
  if (storages.length === 0) return "No exposed storage";

  const enabled = new Set<string>();
  for (const storage of storages) {
    const rules = effectivePolicy(storage, drafts[storage.id]).confirmation_rules;
    if (rules.require_for_write || rules.require_for_overwrite) enabled.add("writes");
    if (rules.require_for_delete || rules.require_for_version_delete) enabled.add("deletes");
    if (rules.require_for_presign) enabled.add("links");
    if (rules.require_for_cross_storage_copy) enabled.add("cross-storage copy");
  }

  if (enabled.size === 0) return "No approvals required";
  return `${Array.from(enabled).join(", ")} require approval`;
}

function assessMcpConnectionSafety({
  exposedStorageCount,
  enabledToolCount,
  showNetworkWarning,
  destructiveAccessEnabled,
}: {
  exposedStorageCount: number;
  enabledToolCount: number;
  showNetworkWarning: boolean;
  destructiveAccessEnabled: boolean;
}): { label: string; description: string; className: string } {
  if (exposedStorageCount === 0) {
    return {
      label: "No storage exposed",
      description: "Agents cannot browse storage until at least one enabled storage is exposed.",
      className: "text-muted-foreground",
    };
  }
  if (enabledToolCount === 0) {
    return {
      label: "No functions enabled",
      description: "Agents can connect, but every MCP tool is currently disabled.",
      className: "text-muted-foreground",
    };
  }
  if (showNetworkWarning) {
    return {
      label: "Review network exposure",
      description: "The HTTP bind address is reachable beyond loopback; use only with intentional network boundaries.",
      className: "text-amber-700 dark:text-amber-300",
    };
  }
  if (destructiveAccessEnabled) {
    return {
      label: "Review destructive access",
      description: "Delete or move tools can run against at least one write-capable storage; keep confirmations enabled unless intentional.",
      className: "text-amber-700 dark:text-amber-300",
    };
  }
  return {
    label: "Ready: scoped local access",
    description: "Current controls expose storage through enabled tools without broad network or destructive-access warnings.",
    className: "text-emerald-700 dark:text-emerald-300",
  };
}

function formatStorageCount(count: number): string {
  return `${count} ${count === 1 ? "storage" : "storages"}`;
}
