import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";

import { FileBrowser, type FileBrowserPaneState } from "@/components/FileBrowser";
import { GlobalSearchDialog } from "@/components/GlobalSearchDialog";
import { StorageSidebar } from "@/components/StorageSidebar";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { TransferQueueProvider } from "@/hooks/use-transfer-queue";
import { toast } from "@/hooks/use-toast";
import {
  addStorage as apiAddStorage,
  approveMcpConfirmation,
  completeOnboarding,
  denyMcpConfirmation,
  exportStorageConfig,
  getAppSettings,
  getMcpClientSnippets,
  getMcpStatus,
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
import { cn } from "@/lib/utils";
import {
  getMcpNotificationPermission,
  notifyPendingMcpConfirmation,
  requestMcpNotificationPermission,
  type McpNotificationPermission,
} from "@/lib/mcpNotifications";
import type {
  AppSettings,
  McpClientSnippets,
  McpAuditEvent,
  McpRuntimeStatus,
  McpSettings,
  McpStoragePolicy,
  McpToolDefinition,
  PendingMcpConfirmation,
  StorageBackend,
  StorageConfig,
  StorageDraft,
  StorageType,
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
const FirstRunOnboardingDialog = lazy(() =>
  import("@/components/FirstRunOnboardingDialog").then((module) => ({
    default: module.FirstRunOnboardingDialog,
  })),
);
const StorageConfigEditorDialog = lazy(() =>
  import("@/components/StorageConfigEditorDialog").then((module) => ({
    default: module.StorageConfigEditorDialog,
  })),
);

const BACKEND_TO_TYPE: Record<StorageBackend, StorageType> = {
  local: "local-fs",
  s3: "aws-s3",
  azure_blob: "azure-blob",
  webdav: "webdav",
  gcs: "gcs",
};

function mapWireStorage(storage: StorageRecordWire): StorageConfig {
  return {
    id: storage.id,
    name: storage.name,
    backend: storage.backend,
    type: BACKEND_TO_TYPE[storage.backend] ?? "local-fs",
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
    },
    runningHttp: status.runningHttp,
    endpoint: status.endpoint,
    endpointDisplay: status.endpointDisplay,
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
    default_access: "read_write",
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
}

function mapPolicyWire(value: unknown): McpStoragePolicy {
  if (!isRecord(value)) return defaultMcpPolicy();
  const fallback = defaultMcpPolicy();
  const confirmationRules = isRecord(value.confirmation_rules)
    ? value.confirmation_rules
    : fallback.confirmation_rules;
  return {
    default_access:
      value.default_access === "none" ||
      value.default_access === "read_only" ||
      value.default_access === "read_write"
        ? value.default_access
        : fallback.default_access,
    allowed_paths: Array.isArray(value.allowed_paths)
      ? value.allowed_paths.filter((path): path is string => typeof path === "string")
      : [],
    denied_paths: Array.isArray(value.denied_paths)
      ? value.denied_paths.filter((path): path is string => typeof path === "string")
      : [],
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
}

interface McpRuntimeStatusWire {
  settings: McpSettingsWire;
  runningHttp: boolean;
  endpoint: string | null;
  endpointDisplay: string;
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
  const [isMcpDialogOpen, setIsMcpDialogOpen] = useState(false);
  const [mcpStatus, setMcpStatus] = useState<McpRuntimeStatus | null>(null);
  const [mcpSnippets, setMcpSnippets] = useState<McpClientSnippets | null>(null);
  const [mcpTools, setMcpTools] = useState<McpToolDefinition[]>([]);
  const [mcpAuditEvents, setMcpAuditEvents] = useState<McpAuditEvent[]>([]);
  const [pendingMcpConfirmations, setPendingMcpConfirmations] = useState<
    PendingMcpConfirmation[]
  >([]);
  const notifiedMcpConfirmationIds = useRef<Set<string>>(new Set());
  const [mcpNotificationPermission, setMcpNotificationPermission] =
    useState<McpNotificationPermission>(() => getMcpNotificationPermission());
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(false);
  const [isPreviewVisible, setIsPreviewVisible] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [isDualPaneOpen, setIsDualPaneOpen] = useState(false);
  const [secondaryStorageId, setSecondaryStorageId] = useState<string | null>(null);
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
      const [auditEvents, pendingConfirmations] = await Promise.all([
        listMcpAuditEvents(200),
        listPendingMcpConfirmations(),
      ]);
      setMcpAuditEvents(auditEvents);
      setPendingMcpConfirmations(pendingConfirmations);
    } catch (error) {
      console.error("Failed to load MCP status", error);
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
      } catch (error) {
        console.error("Failed to load app settings", error);
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
        const pendingConfirmations = await listPendingMcpConfirmations();
        if (!cancelled) {
          setPendingMcpConfirmations(pendingConfirmations);
        }
      } catch (error) {
        console.error("Failed to refresh pending MCP confirmations", error);
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
      await apiUpdateStorage(id, mapDraftForBackend(draft));
      await reloadStorages();
      setStorageRefreshTick((current) => ({
        ...current,
        [id]: (current[id] ?? 0) + 1,
      }));
      toast({
        title: "Storage updated",
        description: `Successfully updated "${draft.name}".`,
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
        await apiRemoveStorage(id);
        await reloadStorages();
        toast({
          title: "Storage deleted",
          description: `${storage?.name ?? "Storage"} has been deleted.`,
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
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,application/json";
    input.onchange = (event) => {
      const file = (event.target as HTMLInputElement).files?.[0];
      if (!file) return;

      const reader = new FileReader();
      reader.onload = (loadEvent) => {
        const text = (loadEvent.target?.result as string) ?? "";
        void (async () => {
          try {
            const result = await importStorageConfig({
              json: text,
              mode: "replace",
              onConflict: "overwrite",
            });
            await reloadStorages();
            toast({
              title: "Import successful",
              description: `Imported ${result.imported} storage configuration(s).`,
            });
          } catch (error: unknown) {
            toast({
              title: "Import failed",
              description: error instanceof Error ? error.message : String(error),
              variant: "destructive",
            });
          }
        })();
      };
      reader.readAsText(file);
    };
    input.click();
  };

  const handleExportStorages = () => {
    void (async () => {
      try {
        const result = await exportStorageConfig(true);
        const blob = new Blob([result.json], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = "infimount-storages.json";
        link.click();
        URL.revokeObjectURL(url);

        toast({
          title: "Export successful",
          description: `Exported ${storages.length} storage configuration(s).`,
        });
      } catch (error: unknown) {
        toast({
          title: "Export failed",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      }
    })();
  };

  const loadStorageConfigJson = async () => {
    const result = await exportStorageConfig(true);
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
        description: `Applied ${result.imported} storage configuration(s) from JSON.`,
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

  const handleSaveMcpSettings = async (settings: McpSettings) => {
    try {
      const status = await updateMcpSettings({
        enabled: settings.enabled,
        transport: settings.transport,
        bindAddress: settings.bindAddress,
        port: settings.port,
        enabledTools: settings.enabledTools,
      });
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

  const refreshMcpActivity = async () => {
    const [auditEvents, pendingConfirmations] = await Promise.all([
      listMcpAuditEvents(200),
      listPendingMcpConfirmations(),
    ]);
    setMcpAuditEvents(auditEvents);
    setPendingMcpConfirmations(pendingConfirmations);
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
  const secondaryStorage =
    storages.find((storage) => storage.id === secondaryStorageId) ?? currentStorage ?? storages[0];

  useEffect(() => {
    if (!secondaryStorageId || !storages.some((storage) => storage.id === secondaryStorageId)) {
      setSecondaryStorageId(currentStorage?.id ?? storages[0]?.id ?? null);
    }
  }, [currentStorage?.id, secondaryStorageId, storages]);

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
            isLoading={isStoragesLoading}
          />
        </div>

        <ResizablePanel className="flex-1 overflow-hidden">
          <div className="flex h-full flex-col">
            {currentStorage ? (
              isDualPaneOpen && secondaryStorage ? (
                <ResizablePanelGroup direction="horizontal" className="h-full">
                  <ResizablePanel defaultSize="50" minSize="25" className="overflow-hidden">
                    <FileBrowser
                      sourceId={currentStorage.id}
                      storageName={currentStorage.name}
                      refreshTick={storageRefreshTick[currentStorage.id] ?? 0}
                      onPreviewVisibilityChange={setIsPreviewVisible}
                      onToggleSidebar={toggleSidebar}
                      isSidebarOpen={isSidebarOpen}
                      onToggleDualPane={() => setIsDualPaneOpen(false)}
                      isDualPane
                      paneTransferTarget={{
                        sourceId: secondaryStorage.id,
                        storageName: secondaryStorage.name,
                        currentPath:
                          secondaryPaneState?.sourceId === secondaryStorage.id
                            ? secondaryPaneState.currentPath
                            : "/",
                        direction: "right",
                      }}
                      onPaneStateChange={setPrimaryPaneState}
                      onTransferCompleted={refreshStorages}
                    />
                  </ResizablePanel>
                  <ResizableHandle className="flex w-px flex-col items-center justify-center bg-border/50" />
                  <ResizablePanel defaultSize="50" minSize="25" className="overflow-hidden">
                    <div className="flex h-full flex-col border-l border-border/40 bg-background">
                      <div className="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-border/70 bg-muted/30 px-3">
                        <label className="text-xs text-muted-foreground" htmlFor="secondary-storage-select">
                          Destination pane
                        </label>
                        <select
                          id="secondary-storage-select"
                          value={secondaryStorage.id}
                          onChange={(event) => setSecondaryStorageId(event.target.value)}
                          className="h-7 max-w-[220px] rounded-md border border-border bg-background px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        >
                          {storages.map((storage) => (
                            <option key={storage.id} value={storage.id}>
                              {storage.name}
                            </option>
                          ))}
                        </select>
                      </div>
                      <FileBrowser
                        sourceId={secondaryStorage.id}
                        storageName={secondaryStorage.name}
                        refreshTick={storageRefreshTick[secondaryStorage.id] ?? 0}
                        showWindowControls={false}
                        showTransferQueue={false}
                        onToggleDualPane={() => setIsDualPaneOpen(false)}
                        isDualPane
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
              ) : (
                <FileBrowser
                  sourceId={currentStorage.id}
                  storageName={currentStorage.name}
                  refreshTick={storageRefreshTick[currentStorage.id] ?? 0}
                  onPreviewVisibilityChange={setIsPreviewVisible}
                  onToggleSidebar={toggleSidebar}
                  isSidebarOpen={isSidebarOpen}
                  onToggleDualPane={() => setIsDualPaneOpen(true)}
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
          <FirstRunOnboardingDialog
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
            onTestConnection={handleTestMcpConnection}
            onComplete={handleCompleteOnboarding}
            onSkip={handleSkipOnboarding}
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
            notificationPermission={mcpNotificationPermission}
            onSave={handleSaveMcpSettings}
            onStartHttp={handleStartMcpHttp}
            onStopHttp={handleStopMcpHttp}
            onTestServer={handleTestMcpConnection}
            onRefreshAudit={refreshMcpActivity}
            onClearAudit={handleClearMcpAudit}
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
        </Suspense>
      </div>
    </TransferQueueProvider>
  );
};

export default Index;
