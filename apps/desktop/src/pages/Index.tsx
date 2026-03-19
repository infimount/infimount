import { useCallback, useEffect, useState } from "react";

import { AddStorageDialog } from "@/components/AddStorageDialog";
import { FileBrowser } from "@/components/FileBrowser";
import { McpSettingsDialog } from "@/components/McpSettingsDialog";
import { StorageConfigEditorDialog } from "@/components/StorageConfigEditorDialog";
import { StorageSidebar } from "@/components/StorageSidebar";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { toast } from "@/hooks/use-toast";
import {
  addStorage as apiAddStorage,
  exportStorageConfig,
  getMcpClientSnippets,
  getMcpStatus,
  listMcpTools,
  importStorageConfig,
  listStorages,
  removeStorage as apiRemoveStorage,
  startMcpHttp,
  stopMcpHttp,
  updateMcpSettings,
  updateStorage as apiUpdateStorage,
  verifyStorage as apiVerifyStorage,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpToolDefinition,
  StorageBackend,
  StorageConfig,
  StorageDraft,
  StorageType,
  StorageValidationResult,
} from "@/types/storage";

const SELECTED_STORAGE_KEY = "infimount.selectedStorageId";

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

interface StorageRecordWire {
  id: string;
  name: string;
  backend: StorageBackend;
  config: unknown;
  enabled: boolean;
  mcp_exposed: boolean;
  read_only: boolean;
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
  const [isMcpDialogOpen, setIsMcpDialogOpen] = useState(false);
  const [mcpStatus, setMcpStatus] = useState<McpRuntimeStatus | null>(null);
  const [mcpSnippets, setMcpSnippets] = useState<McpClientSnippets | null>(null);
  const [mcpTools, setMcpTools] = useState<McpToolDefinition[]>([]);
  const [isPreviewVisible, setIsPreviewVisible] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);

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
    } catch (error) {
      console.error("Failed to load MCP status", error);
    }
  }, []);

  const reloadStorages = useCallback(async () => {
    setIsStoragesLoading(true);
    try {
      const items = await listStorages();
      const mapped = items.map((item) =>
        mapWireStorage(item as unknown as StorageRecordWire),
      );
      setStorages(mapped);

      const storedSelection =
        typeof window === "undefined"
          ? null
          : window.localStorage.getItem(SELECTED_STORAGE_KEY);
      let nextSelection = selectedStorage;
      if (nextSelection && mapped.find((storage) => storage.id === nextSelection)) {
        // keep current selection
      } else if (storedSelection && mapped.find((storage) => storage.id === storedSelection)) {
        nextSelection = storedSelection;
      } else {
        nextSelection = mapped[0]?.id ?? null;
      }
      if (nextSelection !== selectedStorage) {
        setSelectedStorage(nextSelection);
      }
    } catch (error: unknown) {
      toast({
        title: "Failed to load storages",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsStoragesLoading(false);
    }
  }, [selectedStorage]);

  useEffect(() => {
    void reloadStorages();
    void reloadMcpStatus();
  }, [reloadMcpStatus, reloadStorages]);

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

  const currentStorage = storages.find((storage) => storage.id === selectedStorage);

  const toggleSidebar = () => setIsSidebarOpen((current) => !current);
  const closeSidebar = () => setIsSidebarOpen(false);
  const handleSelectStorage = (id: string) => {
    setSelectedStorage(id);
    if (window.matchMedia("(max-width: 767px)").matches) {
      setIsSidebarOpen(false);
    }
  };

  return (
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
            isLoading={isStoragesLoading}
          />
        </div>

        <ResizablePanel className="flex-1 overflow-hidden">
          <div className="flex h-full flex-col">
            {currentStorage ? (
              <FileBrowser
                key={currentStorage.id}
                sourceId={currentStorage.id}
                storageName={currentStorage.name}
                refreshTick={storageRefreshTick[currentStorage.id] ?? 0}
                onPreviewVisibilityChange={setIsPreviewVisible}
                onToggleSidebar={toggleSidebar}
                isSidebarOpen={isSidebarOpen}
              />
            ) : (
              <div className="flex h-full items-center justify-center">
                <p className="text-muted-foreground">Select a storage to view files</p>
              </div>
            )}
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>

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

      <McpSettingsDialog
        open={isMcpDialogOpen}
        onOpenChange={setIsMcpDialogOpen}
        status={mcpStatus}
        snippets={mcpSnippets}
        tools={mcpTools}
        onSave={handleSaveMcpSettings}
        onStartHttp={handleStartMcpHttp}
        onStopHttp={handleStopMcpHttp}
      />

      <StorageConfigEditorDialog
        open={isStorageConfigEditorOpen}
        onOpenChange={setIsStorageConfigEditorOpen}
        onLoad={loadStorageConfigJson}
        onSave={handleSaveStorageConfigJson}
      />
    </div>
  );
};

export default Index;
