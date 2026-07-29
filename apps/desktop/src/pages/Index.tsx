import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";

import { PanelLeft, PanelRight, X } from "lucide-react";

import { AgentWorkspacesDialog } from "@/components/AgentWorkspacesDialog";
import { FileBrowser, type FileBrowserPaneState } from "@/components/FileBrowser";
import { WindowControls } from "@/components/WindowControls";
import { GlobalSearchDialog } from "@/components/GlobalSearchDialog";
import { StorageSidebar } from "@/components/StorageSidebar";
import { Button } from "@/components/ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { TransferQueueProvider } from "@/hooks/use-transfer-queue";
import { toast } from "@/hooks/use-toast";
import {
  addStorage as apiAddStorage,
  approveMcpConfirmation,
  completeOnboarding,
  denyMcpConfirmation,
  exportMcpAuditBundle,
  exportShareableConfig,
  getAppSettings,
  getMcpClientSnippets,
  getMcpStatus,
  listActiveMcpSessions,
  saveWizardState,
  listPendingMcpConfirmations,
  listMcpAuditEvents,
  listMcpTools,
  importStorageConfig,
  listStorages,
  removeStorage as apiRemoveStorage,
  clearMcpAuditEvents,
  skipOnboarding,
  startMcpHttp,
  stopMcpHttp,
  updateMcpSettings,
  updateMcpStoragePolicy,
  updateStorage as apiUpdateStorage,
  verifyStorage as apiVerifyStorage,
} from "@/lib/api";
import { backendToStorageType } from "@/lib/storageMapping";
import { cn } from "@/lib/utils";
import {
  getMcpNotificationPermission,
  notifyPendingMcpConfirmation,
  requestMcpNotificationPermission,
  type McpNotificationPermission,
} from "@/lib/mcpNotifications";
import type {
  ActiveMcpSession,
  AppSettings,
  McpClientSnippets,
  McpAuditEvent,
  McpRuntimeStatus,
  McpSettings,
  McpSettingsUpdate,
  McpStoragePolicy,
  McpToolDefinition,
  PendingMcpConfirmation,
  StorageBackend,
  StorageConfig,
  StorageDraft,
  StorageValidationResult,
} from "@/types/storage";

const SELECTED_STORAGE_KEY = "infimount.selectedStorageId";

const AddStorageDialog = lazy(() =>
  import("@/components/AddStorageDialog").then((module) => ({
    default: module.AddStorageDialog,
  })),
);
const McpSettingsDialog = lazy(() =>
  import("@/components/McpSettingsDialog").then((module) => ({
    default: module.McpSettingsDialog,
  })),
);
const ActivationWizard = lazy(() =>
  import("@/components/ActivationWizard").then((module) => ({
    default: module.ActivationWizard,
  })),
);
const StorageConfigEditorDialog = lazy(() =>
  import("@/components/StorageConfigEditorDialog").then((module) => ({
    default: module.StorageConfigEditorDialog,
  })),
);
const StorageImportDialog = lazy(() =>
  import("@/components/StorageImportDialog").then((module) => ({
    default: module.StorageImportDialog,
  })),
);
const RecoveryBackupDialog = lazy(() =>
  import("@/components/RecoveryBackupDialog").then((module) => ({
    default: module.RecoveryBackupDialog,
  })),
);

function mapWireStorage(storage: StorageRecordWire): StorageConfig {
  return {
    id: storage.id,
    name: storage.name,
    backend: storage.backend,
    type: backendToStorageType(storage.backend),
    config: isRecord(storage.config) ? storage.config : {},
    enabled: storage.enabled,
    mcpExposed: storage.mcp_exposed,
    readOnly: storage.read_only,
    connected: true,
    createdAt: storage.created_at,
    updatedAt: storage.updated_at,
    mcpPolicy: mapPolicyWire(storage.mcp_policy),
  };
}

function mapStatusWire(status: McpRuntimeStatusWire): McpRuntimeStatus {
  return {
    settings: {
      enabled: status.settings.enabled,
      transport: status.settings.transport,
      bindAddress: status.settings.bindAddress,
      port: status.settings.port,
      enabledTools: status.settings.enabledTools ?? [],
      securityBaselineVersion: status.settings.securityBaselineVersion ?? 2,
      authTokenConfigured: status.authTokenConfigured,
    },
    runningHttp: status.runningHttp,
    endpoint: status.endpoint,
    endpointDisplay: status.endpointDisplay,
    authTokenConfigured: status.authTokenConfigured,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function mapDraftForBackend(draft: StorageDraft): StorageDraft {
  return {
    ...draft,
    config: draft.config,
  };
}

function defaultMcpPolicy(): McpStoragePolicy {
  return {
    version: 2,
    default_access: "none",
    rules: [],
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
}

function mapPolicyWire(value: unknown): McpStoragePolicy {
  if (!isRecord(value)) return defaultMcpPolicy();
  const fallback = defaultMcpPolicy();
  const confirmationRules = isRecord(value.confirmation_rules)
    ? value.confirmation_rules
    : fallback.confirmation_rules;
  const version: number =
    typeof value.version === "number" ? value.version : 1;
  const denied_paths = Array.isArray(value.denied_paths)
    ? value.denied_paths.filter((path): path is string => typeof path === "string")
    : [];
  const allowed_paths = Array.isArray(value.allowed_paths)
    ? value.allowed_paths.filter((path): path is string => typeof path === "string")
    : [];
  const default_access: McpStoragePolicy["default_access"] =
    value.default_access === "none" ||
    value.default_access === "read_only" ||
    value.default_access === "read_write"
      ? value.default_access
      : fallback.default_access;
  let rules: McpStoragePolicy["rules"] = Array.isArray(value.rules)
    ? value.rules.filter(
        (r): r is McpStoragePolicy["rules"][number] =>
          isRecord(r) && typeof r.id === "string" && typeof r.prefix === "string",
      )
    : [];
  if (version < 2) {
    if (allowed_paths.length > 0) {
      rules = allowed_paths.map((path, i) => ({
        id: `migrated-${i}`,
        prefix: path,
        access: default_access === "none" ? "read_only" : default_access,
        source: { kind: "manual" },
      }));
      return {
        version: 2,
        default_access: "none",
        rules,
        denied_paths,
        confirmation_rules: { ...fallback.confirmation_rules, ...confirmationRules },
      };
    }
    return {
      version: 2,
      default_access, // preserve original default when no allowed_paths to migrate
      rules,
      denied_paths,
      confirmation_rules: { ...fallback.confirmation_rules, ...confirmationRules },
    };
  }
  return {
    version: 2,
    default_access,
    rules,
    denied_paths,
    confirmation_rules: {
      require_for_write:
        typeof confirmationRules.require_for_write === "boolean"
          ? confirmationRules.require_for_write
          : true,
      require_for_overwrite:
        typeof confirmationRules.require_for_overwrite === "boolean"
          ? confirmationRules.require_for_overwrite
          : true,
      require_for_delete:
        typeof confirmationRules.require_for_delete === "boolean"
          ? confirmationRules.require_for_delete
          : true,
      require_for_version_delete:
        typeof confirmationRules.require_for_version_delete === "boolean"
          ? confirmationRules.require_for_version_delete
          : true,
      require_for_presign:
        typeof confirmationRules.require_for_presign === "boolean"
          ? confirmationRules.require_for_presign
          : true,
      require_for_cross_storage_copy:
        typeof confirmationRules.require_for_cross_storage_copy === "boolean"
          ? confirmationRules.require_for_cross_storage_copy
          : true,
    },
  };
}

interface StorageRecordWire {
  id: string;
  name: string;
  backend: StorageBackend;
  config: unknown;
  enabled: boolean;
  mcp_exposed: boolean;
  read_only: boolean;
  mcp_policy?: unknown;
  created_at: string;
  updated_at: string;
}

interface McpSettingsWire {
  enabled: boolean;
  transport: McpSettings["transport"];
  bindAddress: string;
  port: number;
  enabledTools?: string[];
  securityBaselineVersion?: number;
}

interface McpRuntimeStatusWire {
  settings: McpSettingsWire;
  runningHttp: boolean;
  endpoint: string | null;
  endpointDisplay: string;
  authTokenConfigured: boolean;
}

const Index = () => {
  const [storages, setStorages] = useState<StorageConfig[]>([]);
  const [isStoragesLoading, setIsStoragesLoading] = useState(true);
  const [storageRefreshTick, setStorageRefreshTick] = useState<Record<string, number>>({});
  const [selectedStorage, setSelectedStorage] = useState<string | null>(() => {
    if (typeof window === "undefined") return null;
    return window.localStorage.getItem(SELECTED_STORAGE_KEY);
  });
  const [isAddDialogOpen, setIsAddDialogOpen] = useState(false);
  const [editingStorage, setEditingStorage] = useState<StorageConfig | null>(null);
  const [isStorageConfigEditorOpen, setIsStorageConfigEditorOpen] = useState(false);
  const [isGlobalSearchOpen, setIsGlobalSearchOpen] = useState(false);
  const [isAgentWorkspacesOpen, setIsAgentWorkspacesOpen] = useState(false);
  const [isMcpDialogOpen, setIsMcpDialogOpen] = useState(false);
  const [isImportDialogOpen, setIsImportDialogOpen] = useState(false);
  const [isBackupDialogOpen, setIsBackupDialogOpen] = useState(false);
  const [mcpStatus, setMcpStatus] = useState<McpRuntimeStatus | null>(null);
  const [mcpSnippets, setMcpSnippets] = useState<McpClientSnippets | null>(null);
  const [mcpTools, setMcpTools] = useState<McpToolDefinition[]>([]);
  const [mcpAuditEvents, setMcpAuditEvents] = useState<McpAuditEvent[]>([]);
  const [pendingMcpConfirmations, setPendingMcpConfirmations] = useState<
    PendingMcpConfirmation[]
  >([]);
  const [activeMcpSessions, setActiveMcpSessions] = useState<ActiveMcpSession[]>([]);
  const notifiedMcpConfirmationIds = useRef<Set<string>>(new Set());
  const [mcpNotificationPermission, setMcpNotificationPermission] =
    useState<McpNotificationPermission>(() => getMcpNotificationPermission());
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(false);
  const [isPreviewVisible, setIsPreviewVisible] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [isDualPaneOpen, setIsDualPaneOpen] = useState(false);
  const [primaryPaneState, setPrimaryPaneState] = useState<FileBrowserPaneState | null>(null);
  const [secondaryPaneState, setSecondaryPaneState] = useState<FileBrowserPaneState | null>(null);

  const reloadMcpStatus = useCallback(async () => {
    try {
      const [status, snippets, tools] = await Promise.all([
        getMcpStatus().then(mapStatusWire),
        getMcpClientSnippets(),
        listMcpTools(),
      ]);
      setMcpStatus(status);
      setMcpSnippets(snippets);
      setMcpTools(tools);
      const [auditEvents, pendingConfirmations, activeSessions] = await Promise.all([
        listMcpAuditEvents(200),
        listPendingMcpConfirmations(),
        listActiveMcpSessions(),
      ]);
      setMcpAuditEvents(auditEvents);
      setPendingMcpConfirmations(pendingConfirmations);
      setActiveMcpSessions(activeSessions);
    } catch {
      // The settings dialog renders its unavailable state without exposing backend details.
    }
  }, []);

  const reloadStorages = useCallback(async () => {
    setIsStoragesLoading(true);
    try {
      const items = await listStorages();
      const mapped = items.map((item) => mapWireStorage(item as unknown as StorageRecordWire));
      setStorages(mapped);

      const storedSelection =
        typeof window === "undefined" ? null : window.localStorage.getItem(SELECTED_STORAGE_KEY);
      setSelectedStorage((currentSelection) => {
        if (currentSelection && mapped.find((storage) => storage.id === currentSelection)) {
          return currentSelection;
        }
        if (storedSelection && mapped.find((storage) => storage.id === storedSelection)) {
          return storedSelection;
        }
        return mapped[0]?.id ?? null;
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to load storages",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsStoragesLoading(false);
    }
  }, []);

  useEffect(() => {
    void reloadStorages();
  }, [reloadStorages]);

  useEffect(() => {
    void (async () => {
      try {
        setAppSettings(await getAppSettings());
      } catch {
        // Keep the safe in-memory defaults when local settings cannot be loaded.
      }
    })();
  }, []);

  useEffect(() => {
    if (isStoragesLoading || !appSettings) return;
    const onboardingDone = appSettings.onboardingCompleted || appSettings.onboardingSkipped;
    setIsOnboardingOpen(!onboardingDone && storages.length === 0);
  }, [appSettings, isStoragesLoading, storages.length]);

  useEffect(() => {
    if (!isMcpDialogOpen) return;
    void reloadMcpStatus();
  }, [isMcpDialogOpen, reloadMcpStatus]);

  useEffect(() => {
    if (!mcpStatus?.runningHttp) return;

    let cancelled = false;
    const refreshPendingConfirmations = async () => {
      try {
        const [pendingConfirmations, activeSessions] = await Promise.all([
          listPendingMcpConfirmations(),
          listActiveMcpSessions(),
        ]);
        if (!cancelled) {
          setPendingMcpConfirmations(pendingConfirmations);
          setActiveMcpSessions(activeSessions);
        }
      } catch {
        // Retain the last successfully loaded local approval state.
      }
    };

    void refreshPendingConfirmations();
    const interval = window.setInterval(() => void refreshPendingConfirmations(), 5000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [mcpStatus?.runningHttp]);

  useEffect(() => {
    for (const pending of pendingMcpConfirmations) {
      if (notifiedMcpConfirmationIds.current.has(pending.operation_id)) {
        continue;
      }

      notifiedMcpConfirmationIds.current.add(pending.operation_id);
      notifyPendingMcpConfirmation(pending, () => setIsMcpDialogOpen(true));
      toast({
        title: "MCP approval needed",
        description: `${pending.tool_name} wants ${pending.risk_type} access on ${pending.storage_name}.`,
      });
    }
  }, [pendingMcpConfirmations]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (selectedStorage) {
      window.localStorage.setItem(SELECTED_STORAGE_KEY, selectedStorage);
    } else {
      window.localStorage.removeItem(SELECTED_STORAGE_KEY);
    }
  }, [selectedStorage]);

  useEffect(() => {
    const mql = window.matchMedia("(max-width: 767px)");
    const handle = () => {
      setIsSidebarOpen(!mql.matches);
    };
    handle();
    mql.addEventListener("change", handle);
    return () => mql.removeEventListener("change", handle);
  }, []);

  useEffect(() => {
    const isCompact = window.matchMedia("(max-width: 1024px)").matches;
    if (isPreviewVisible && isCompact) {
      setIsSidebarOpen(false);
    } else if (!isPreviewVisible && !isCompact) {
      setIsSidebarOpen(true);
    }
  }, [isPreviewVisible]);

  const handleAddStorage = async (draft: StorageDraft) => {
    try {
      const added = (await apiAddStorage(
        mapDraftForBackend(draft),
      )) as unknown as StorageRecordWire;
      await reloadStorages();
      setSelectedStorage(added.id);
      toast({
        title: "Storage added",
        description: `Successfully added "${draft.name}".`,
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to add storage",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    }
  };

  const handleEditStorage = (id: string) => {
    const storage = storages.find((item) => item.id === id) ?? null;
    if (!storage) return;
    setEditingStorage(storage);
    setIsAddDialogOpen(true);
  };

  const handleUpdateStorage = async (id: string, draft: StorageDraft) => {
    try {
      const result = await apiUpdateStorage(id, mapDraftForBackend(draft));
      await reloadStorages();
      setStorageRefreshTick((current) => ({
        ...current,
        [id]: (current[id] ?? 0) + 1,
      }));
      toast({
        title: "Storage updated",
        description: result.warning
          ? `Successfully updated "${draft.name}". ${result.warning}`
          : `Successfully updated "${draft.name}".`,
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to update storage",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    } finally {
      setEditingStorage(null);
    }
  };

  const handleVerifyStorage = async (draft: StorageDraft): Promise<StorageValidationResult> => {
    return apiVerifyStorage(mapDraftForBackend(draft));
  };

  const handleDeleteStorage = (id: string) => {
    const storage = storages.find((item) => item.id === id);
    void (async () => {
      try {
        const removal = await apiRemoveStorage(id);
        await reloadStorages();
        toast({
          title: removal.warning ? "Storage deleted with cleanup pending" : "Storage deleted",
          description:
            removal.warning ?? `${storage?.name ?? "Storage"} has been deleted.`,
          variant: "destructive",
        });
      } catch (error: unknown) {
        toast({
          title: "Failed to delete storage",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      }
    })();
  };

  const handleRefreshStorage = (id: string) => {
    const storage = storages.find((item) => item.id === id);
    void (async () => {
      toast({
        title: "Refreshing",
        description: `Refreshing ${storage?.name ?? "storage"}...`,
      });
      setStorageRefreshTick((current) => ({
        ...current,
        [id]: (current[id] ?? 0) + 1,
      }));
      await reloadStorages();
    })();
  };

  const handleImportStorages = () => {
    setIsImportDialogOpen(true);
  };

  const handleExportStorages = () => {
    setIsBackupDialogOpen(true);
  };

  const loadStorageConfigJson = async () => {
    const result = await exportShareableConfig();
    return result.json;
  };

  const handleSaveStorageConfigJson = async (json: string) => {
    try {
      const result = await importStorageConfig({
        json,
        mode: "replace",
        onConflict: "overwrite",
      });
      await reloadStorages();
      toast({
        title: "Storage config updated",
        description: result.warnings?.length
          ? `Applied ${result.imported} storage configuration(s). ${result.warnings.join(" ")}`
          : `Applied ${result.imported} storage configuration(s) from JSON.`,
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to apply storage config",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    }
  };

  const handleSaveMcpSettings = async (settings: McpSettingsUpdate) => {
    try {
      const status = await updateMcpSettings(settings);
      setMcpStatus(mapStatusWire(status as unknown as McpRuntimeStatusWire));
      setMcpSnippets(await getMcpClientSnippets());
    } catch (error: unknown) {
      toast({
        title: "Failed to update MCP settings",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    }
  };

  const handleStartMcpHttp = async () => {
    try {
      const status = await startMcpHttp();
      setMcpStatus(mapStatusWire(status as unknown as McpRuntimeStatusWire));
      setMcpSnippets(await getMcpClientSnippets());
    } catch (error: unknown) {
      toast({
        title: "Failed to start MCP HTTP server",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    }
  };

  const handleStopMcpHttp = async () => {
    try {
      const status = await stopMcpHttp();
      setMcpStatus(mapStatusWire(status as unknown as McpRuntimeStatusWire));
      setMcpSnippets(await getMcpClientSnippets());
    } catch (error: unknown) {
      toast({
        title: "Failed to stop MCP HTTP server",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
      throw error;
    }
  };

  const handleCompleteOnboarding = async () => {
    const next = await completeOnboarding();
    setAppSettings(next);
    setIsOnboardingOpen(false);
  };

  const handleSkipOnboarding = async () => {
    const next = await skipOnboarding();
    setAppSettings(next);
    setIsOnboardingOpen(false);
  };

  const handleSaveWizardState = async (
    step: string | null,
    completedSteps: string[],
  ) => {
    const next = await saveWizardState({ step, completedSteps });
    setAppSettings(next);
  };

  const handleTestMcpConnection = async () => {
    await reloadMcpStatus();
    const exposedCount = storages.filter((storage) => storage.enabled && storage.mcpExposed).length;
    toast({
      title: "MCP setup check",
      description: `${mcpTools.length} function(s) available. ${exposedCount} storage(s) currently exposed.`,
    });
  };

  const handleClearMcpAudit = async () => {
    await clearMcpAuditEvents();
    setMcpAuditEvents([]);
    toast({
      title: "Audit log cleared",
      description: "Local MCP audit events have been removed.",
    });
  };

  const handleExportMcpAudit = async (events: McpAuditEvent[]) => {
    const result = await exportMcpAuditBundle(events);
    toast({
      title: "Audit bundle exported",
      description: `${result.eventCount} event(s) written to ${result.path}`,
    });
  };

  const refreshMcpActivity = async () => {
    const [auditEvents, pendingConfirmations, activeSessions] = await Promise.all([
      listMcpAuditEvents(200),
      listPendingMcpConfirmations(),
      listActiveMcpSessions(),
    ]);
    setMcpAuditEvents(auditEvents);
    setPendingMcpConfirmations(pendingConfirmations);
    setActiveMcpSessions(activeSessions);
  };

  const handleApproveMcpConfirmation = async (operationId: string) => {
    await approveMcpConfirmation(operationId);
    await refreshMcpActivity();
    toast({
      title: "MCP operation approved",
      description: "The waiting agent can retry with the approved operation ID.",
    });
  };

  const handleDenyMcpConfirmation = async (operationId: string) => {
    await denyMcpConfirmation(operationId);
    await refreshMcpActivity();
    toast({
      title: "MCP operation denied",
      description: "The pending operation was removed from the approval queue.",
    });
  };

  const handleEnableMcpNotifications = async () => {
    const permission = await requestMcpNotificationPermission();
    setMcpNotificationPermission(permission);
    toast({
      title:
        permission === "granted" ? "Desktop notifications enabled" : "Notifications unavailable",
      description:
        permission === "granted"
          ? "Infimount will notify you when risky MCP operations need approval."
          : "Infimount could not enable desktop notifications in this environment.",
      variant: permission === "granted" ? "success" : "destructive",
    });
  };

  const handleUpdateMcpStoragePolicy = async (
    storageId: string,
    policy: McpStoragePolicy,
  ) => {
    const updated = await updateMcpStoragePolicy(storageId, policy);
    const mapped = mapWireStorage(updated as unknown as StorageRecordWire);
    setStorages((current) => current.map((storage) => (storage.id === storageId ? mapped : storage)));
    toast({
      title: "MCP policy updated",
      description: "Path rules will apply to new MCP requests immediately.",
    });
  };

  const currentStorage = storages.find((storage) => storage.id === selectedStorage);
  const secondaryStorage = currentStorage;

  const refreshStorages = useCallback((storageIds: string[]) => {
    setStorageRefreshTick((current) => {
      const next = { ...current };
      for (const storageId of storageIds) {
        next[storageId] = (next[storageId] ?? 0) + 1;
      }
      return next;
    });
  }, []);

  const toggleSidebar = () => setIsSidebarOpen((current) => !current);
  const closeSidebar = () => setIsSidebarOpen(false);
  const openDualPane = () => {
    setSecondaryPaneState(null);
    setIsDualPaneOpen(true);
  };
  const closeDualPane = () => {
    setSecondaryPaneState(null);
    setIsDualPaneOpen(false);
  };
  const handleSelectStorage = (id: string) => {
    setSelectedStorage(id);
    if (window.matchMedia("(max-width: 767px)").matches) {
      setIsSidebarOpen(false);
    }
  };

  return (
    <TransferQueueProvider>
      <div className="flex h-screen w-full overflow-hidden rounded-[12px] border border-border/40 bg-background">
        <ResizablePanelGroup direction="horizontal">
        {isSidebarOpen ? (
          <>
            <ResizablePanel
              className="hidden md:block transition-all duration-200"
              defaultSize="20%"
              minSize="15%"
              maxSize="40%"
            >
              <StorageSidebar
                storages={storages}
                selectedStorage={selectedStorage}
                onSelectStorage={handleSelectStorage}
                onAddStorage={() => setIsAddDialogOpen(true)}
                onEditStorage={handleEditStorage}
                onDeleteStorage={handleDeleteStorage}
                onRefreshStorage={handleRefreshStorage}
                onImportStorages={handleImportStorages}
                onEditStorageConfig={() => setIsStorageConfigEditorOpen(true)}
                onExportStorages={handleExportStorages}
                onOpenMcpSettings={() => setIsMcpDialogOpen(true)}
                onOpenOnboarding={() => setIsOnboardingOpen(true)}
                onOpenGlobalSearch={() => setIsGlobalSearchOpen(true)}
                onOpenAgentWorkspaces={() => setIsAgentWorkspacesOpen(true)}
                isLoading={isStoragesLoading}
              />
            </ResizablePanel>
            <ResizableHandle className="hidden md:flex w-px flex-col items-center justify-center bg-transparent group/handle relative z-10">
              <div className="absolute inset-y-0 -left-1 -right-1 z-50 cursor-col-resize" />
              <div className="h-full w-[1px] bg-border/40 transition-colors group-hover/handle:bg-primary/40" />
            </ResizableHandle>
          </>
        ) : null}

        <div
          className={cn(
            "fixed inset-0 z-40 bg-black/40 transition-opacity md:hidden",
            isSidebarOpen ? "opacity-100" : "pointer-events-none opacity-0",
          )}
          onClick={closeSidebar}
        />
        <div
          className={cn(
            "fixed inset-y-0 left-0 z-50 w-72 max-w-[85vw] bg-sidebar shadow-xl transition-transform md:hidden",
            isSidebarOpen ? "translate-x-0" : "-translate-x-full",
          )}
        >
          <StorageSidebar
            storages={storages}
            selectedStorage={selectedStorage}
            onSelectStorage={handleSelectStorage}
            onAddStorage={() => setIsAddDialogOpen(true)}
            onEditStorage={handleEditStorage}
            onDeleteStorage={handleDeleteStorage}
            onRefreshStorage={handleRefreshStorage}
            onImportStorages={handleImportStorages}
            onEditStorageConfig={() => setIsStorageConfigEditorOpen(true)}
            onExportStorages={handleExportStorages}
            onOpenMcpSettings={() => setIsMcpDialogOpen(true)}
            onOpenOnboarding={() => setIsOnboardingOpen(true)}
            onOpenGlobalSearch={() => setIsGlobalSearchOpen(true)}
            onOpenAgentWorkspaces={() => setIsAgentWorkspacesOpen(true)}
            isLoading={isStoragesLoading}
          />
        </div>

        <ResizablePanel className="flex-1 overflow-hidden">
          <div className="flex h-full flex-col">
            {currentStorage ? (
              isDualPaneOpen && secondaryStorage ? (
                <div className="flex h-full flex-col overflow-hidden bg-background">
                  <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border/70 bg-muted/30 px-4" data-tauri-drag-region>
                    <div className="flex items-center gap-1 tauri-no-drag">
                      <Button
                        size="icon"
                        variant="ghost"
                        className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                        onClick={toggleSidebar}
                        title={isSidebarOpen ? "Hide Storage Sidebar" : "Show Storage Sidebar"}
                        aria-label={isSidebarOpen ? "Hide Storage Sidebar" : "Show Storage Sidebar"}
                      >
                        {isSidebarOpen ? <PanelRight className="h-4 w-4" /> : <PanelLeft className="h-4 w-4" />}
                      </Button>
                    </div>
                    <div className="min-w-0 flex-1" data-tauri-drag-region>
                      <div className="truncate text-sm font-medium text-foreground">
                        {currentStorage.name}
                      </div>
                      <div className="truncate text-[11px] text-muted-foreground">
                        Split view, two panes in the same storage
                      </div>
                    </div>
                    <div className="flex items-center gap-2 tauri-no-drag">
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-8 gap-1.5 px-2 text-xs text-foreground/75 hover:bg-black/5 dark:hover:bg-white/5"
                        onClick={closeDualPane}
                        aria-label="Close split pane"
                        title="Close split pane"
                      >
                        <X className="h-4 w-4" />
                        Close Split
                      </Button>
                      <div className="ml-1 border-l border-border/50 pl-2">
                        <WindowControls />
                      </div>
                    </div>
                  </div>
                  <ResizablePanelGroup direction="horizontal" className="min-h-0 flex-1">
                    <ResizablePanel defaultSize="50" minSize="25" className="overflow-hidden">
                      <FileBrowser
                        sourceId={currentStorage.id}
                        storageName={currentStorage.name}
                        refreshTick={storageRefreshTick[currentStorage.id] ?? 0}
                        onPreviewVisibilityChange={setIsPreviewVisible}
                        showWindowControls={false}
                        showTransferQueue={false}
                        isDualPane
                        headerVariant="pane"
                        paneLabel="Left"
                        initialPath={
                          primaryPaneState?.sourceId === currentStorage.id
                            ? primaryPaneState.currentPath
                            : "/"
                        }
                        paneTransferTarget={{
                          sourceId: secondaryStorage.id,
                          storageName: secondaryStorage.name,
                          currentPath:
                            secondaryPaneState?.sourceId === secondaryStorage.id
                              ? secondaryPaneState.currentPath
                              : primaryPaneState?.sourceId === currentStorage.id
                                ? primaryPaneState.currentPath
                                : "/",
                          direction: "right",
                        }}
                        onPaneStateChange={setPrimaryPaneState}
                        onTransferCompleted={refreshStorages}
                      />
                    </ResizablePanel>
                    <ResizableHandle className="flex w-px flex-col items-center justify-center bg-transparent group/handle relative z-10">
                      <div className="absolute inset-y-0 -left-1 -right-1 z-50 cursor-col-resize" />
                      <div className="h-full w-[1px] bg-border/50 transition-colors group-hover/handle:bg-primary/40" />
                    </ResizableHandle>
                    <ResizablePanel defaultSize="50" minSize="25" className="overflow-hidden">
                      <div className="flex h-full flex-col border-l border-border/40 bg-background">
                        <FileBrowser
                          sourceId={secondaryStorage.id}
                          storageName={secondaryStorage.name}
                          refreshTick={storageRefreshTick[secondaryStorage.id] ?? 0}
                          showWindowControls={false}
                          showTransferQueue={false}
                          isDualPane
                          headerVariant="pane"
                          paneLabel="Right"
                          initialPath={
                            primaryPaneState?.sourceId === currentStorage.id
                              ? primaryPaneState.currentPath
                              : "/"
                          }
                          paneTransferTarget={{
                            sourceId: currentStorage.id,
                            storageName: currentStorage.name,
                            currentPath:
                              primaryPaneState?.sourceId === currentStorage.id
                                ? primaryPaneState.currentPath
                                : "/",
                            direction: "left",
                          }}
                          onPaneStateChange={setSecondaryPaneState}
                          onTransferCompleted={refreshStorages}
                        />
                      </div>
                    </ResizablePanel>
                  </ResizablePanelGroup>
                </div>
              ) : (
                <FileBrowser
                  sourceId={currentStorage.id}
                  storageName={currentStorage.name}
                  refreshTick={storageRefreshTick[currentStorage.id] ?? 0}
                  onPreviewVisibilityChange={setIsPreviewVisible}
                  onToggleSidebar={toggleSidebar}
                  isSidebarOpen={isSidebarOpen}
                  onToggleDualPane={openDualPane}
                  onPaneStateChange={setPrimaryPaneState}
                  onTransferCompleted={refreshStorages}
                />
              )
            ) : (
              <div className="flex h-full items-center justify-center">
                <p className="text-muted-foreground">Select a storage to view files</p>
              </div>
            )}
          </div>
        </ResizablePanel>
        </ResizablePanelGroup>

        <Suspense fallback={null}>
        {isOnboardingOpen ? (
          <ActivationWizard
            open={isOnboardingOpen}
            onOpenChange={setIsOnboardingOpen}
            onAddStorage={() => {
              setIsOnboardingOpen(false);
              setIsAddDialogOpen(true);
            }}
            onOpenMcpSettings={() => {
              setIsOnboardingOpen(false);
              setIsMcpDialogOpen(true);
            }}
            onComplete={handleCompleteOnboarding}
            onSkip={handleSkipOnboarding}
            onSaveState={handleSaveWizardState}
            storagesCount={storages.length}
            mcpStatus={mcpStatus ?? undefined}
            clientSnippets={mcpSnippets ?? undefined}
          />
        ) : null}

        {isAddDialogOpen ? (
          <AddStorageDialog
            open={isAddDialogOpen}
            onOpenChange={(open) => {
              setIsAddDialogOpen(open);
              if (!open) setEditingStorage(null);
            }}
            onAdd={handleAddStorage}
            onUpdate={handleUpdateStorage}
            onVerify={handleVerifyStorage}
            initialStorage={editingStorage ?? undefined}
          />
        ) : null}

        {isMcpDialogOpen ? (
          <McpSettingsDialog
            open={isMcpDialogOpen}
            onOpenChange={setIsMcpDialogOpen}
            status={mcpStatus}
            snippets={mcpSnippets}
            tools={mcpTools}
            storages={storages}
            auditEvents={mcpAuditEvents}
            pendingConfirmations={pendingMcpConfirmations}
            activeSessions={activeMcpSessions}
            notificationPermission={mcpNotificationPermission}
            onSave={handleSaveMcpSettings}
            onStartHttp={handleStartMcpHttp}
            onStopHttp={handleStopMcpHttp}
            onTestServer={handleTestMcpConnection}
            onRefreshAudit={refreshMcpActivity}
            onClearAudit={handleClearMcpAudit}
            onExportAuditBundle={handleExportMcpAudit}
            onApproveConfirmation={handleApproveMcpConfirmation}
            onDenyConfirmation={handleDenyMcpConfirmation}
            onEnableNotifications={handleEnableMcpNotifications}
            onUpdateStoragePolicy={handleUpdateMcpStoragePolicy}
          />
        ) : null}

        {isStorageConfigEditorOpen ? (
          <StorageConfigEditorDialog
            open={isStorageConfigEditorOpen}
            onOpenChange={setIsStorageConfigEditorOpen}
            onLoad={loadStorageConfigJson}
            onSave={handleSaveStorageConfigJson}
          />
        ) : null}

        {isGlobalSearchOpen ? (
          <GlobalSearchDialog
            open={isGlobalSearchOpen}
            storages={storages}
            onOpenChange={setIsGlobalSearchOpen}
            onSelectStorage={handleSelectStorage}
          />
        ) : null}

        {isAgentWorkspacesOpen ? (
          <AgentWorkspacesDialog
            open={isAgentWorkspacesOpen}
            storages={storages}
            auditEvents={mcpAuditEvents}
            onOpenChange={setIsAgentWorkspacesOpen}
            onSelectStorage={handleSelectStorage}
            onUpdateStoragePolicy={handleUpdateMcpStoragePolicy}
          />
        ) : null}

        {isImportDialogOpen ? (
          <StorageImportDialog
            open={isImportDialogOpen}
            onOpenChange={setIsImportDialogOpen}
            onImportComplete={reloadStorages}
          />
        ) : null}

        {isBackupDialogOpen ? (
          <RecoveryBackupDialog
            open={isBackupDialogOpen}
            onOpenChange={setIsBackupDialogOpen}
            onRestoreComplete={reloadStorages}
          />
        ) : null}
        </Suspense>
      </div>
    </TransferQueueProvider>
  );
};

export default Index;
