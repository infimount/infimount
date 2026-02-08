import { useCallback, useEffect, useState } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { StorageSidebar } from "@/components/StorageSidebar";
import { AddStorageDialog } from "@/components/AddStorageDialog";
import { FileBrowser } from "@/components/FileBrowser";
import { StorageConfig, StorageType } from "@/types/storage";
import type { Source, SourceKind } from "@/types/source";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import {
  listSources,
  addSource as apiAddSource,
  removeSource as apiRemoveSource,
  updateSource as apiUpdateSource,
  replaceSources as apiReplaceSources,
} from "@/lib/api";

const mapSourceKindToStorageType = (kind: SourceKind): StorageType => {
  switch (kind) {
    case "s3":
      return "aws-s3";
    case "azure_blob":
      return "azure-blob";
    case "webdav":
      return "webdav";
    case "gcs":
      return "gcs";
    case "local":
    default:
      return "local-fs";
  }
};

const mapStorageTypeToSourceKind = (type: StorageType): SourceKind => {
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
};

const deriveRootFromConfig = (type: StorageType, config: Record<string, string>): string => {
  if (type === "local-fs") {
    return config.rootPath || "";
  }
  if (type === "aws-s3") {
    const bucket = config.bucketName || "";
    const region = config.region || "";
    return [bucket, region].filter(Boolean).join("@");
  }
  if (type === "azure-blob") {
    const account = config.accountName || "";
    const container = config.containerName || "";
    return [account, container].filter(Boolean).join("/");
  }
  if (type === "webdav") {
    return config.rootPath || config.serverUrl || "";
  }
  if (type === "gcs") {
    return config.bucket || "";
  }
  return "";
};

const sourceToStorage = (source: Source): StorageConfig => ({
  id: source.id,
  name: source.name,
  type: mapSourceKindToStorageType(source.kind),
  config: source.config ?? {},
  connected: true,
});

const storageToSource = (storage: StorageConfig): Source => ({
  id: storage.id,
  name: storage.name,
  kind: mapStorageTypeToSourceKind(storage.type),
  root: deriveRootFromConfig(storage.type, storage.config ?? {}),
  config: storage.config,
});

const SELECTED_STORAGE_KEY = "infimount.selectedStorageId";

const Index = () => {
  const [storages, setStorages] = useState<StorageConfig[]>([]);
  const [isStoragesLoading, setIsStoragesLoading] = useState(true);
  const [storageRefreshTick, setStorageRefreshTick] = useState<Record<string, number>>({});
  const [selectedStorage, setSelectedStorage] = useState<string | null>(() => {
    if (typeof window === "undefined") return null;
    const stored = window.localStorage.getItem(SELECTED_STORAGE_KEY);
    return stored || null;
  });
  const [isAddDialogOpen, setIsAddDialogOpen] = useState(false);
  const [editingStorage, setEditingStorage] = useState<StorageConfig | null>(null);

  const reloadStorages = useCallback(async () => {
    setIsStoragesLoading(true);
    try {
      const sources = await listSources();
      const mapped = sources.map(sourceToStorage);
      setStorages(mapped);
      const storedSelection =
        typeof window === "undefined"
          ? null
          : window.localStorage.getItem(SELECTED_STORAGE_KEY);
      let nextSelection = selectedStorage;
      if (nextSelection && mapped.find((s) => s.id === nextSelection)) {
        // keep current selection
      } else if (storedSelection && mapped.find((s) => s.id === storedSelection)) {
        nextSelection = storedSelection;
      } else {
        nextSelection = mapped[0]?.id ?? null;
      }
      if (nextSelection !== selectedStorage) {
        setSelectedStorage(nextSelection);
      }
    } catch (error: unknown) {
      toast({
        title: "Failed to load sources",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsStoragesLoading(false);
    }
  }, [selectedStorage]);

  useEffect(() => {
    void reloadStorages();
  }, [reloadStorages]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (selectedStorage) {
      window.localStorage.setItem(SELECTED_STORAGE_KEY, selectedStorage);
    } else {
      window.localStorage.removeItem(SELECTED_STORAGE_KEY);
    }
  }, [selectedStorage]);

  const handleAddStorage = async (data: {
    name: string;
    type: StorageType;
    config: Record<string, string>;
  }) => {
    try {
      const newSource: Source = {
        id: Math.random().toString(36).substring(2, 10),
        name: data.name,
        kind: mapStorageTypeToSourceKind(data.type),
        root: deriveRootFromConfig(data.type, data.config),
        config: data.config,
      };
      await apiAddSource(newSource);
      await reloadStorages();
      setSelectedStorage(newSource.id);
      toast({
        title: "Storage added",
        description: `Successfully added "${data.name}".`,
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to add storage",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    }
  };

  const handleEditStorage = (id: string) => {
    const storage = storages.find((s) => s.id === id) || null;
    if (!storage) return;
    setEditingStorage(storage);
    setIsAddDialogOpen(true);
  };

  const handleUpdateStorage = async (
    id: string,
    data: { name: string; type: StorageType; config: Record<string, string> },
  ) => {
    try {
      const updatedSource: Source = {
        id,
        name: data.name,
        kind: mapStorageTypeToSourceKind(data.type),
        root: deriveRootFromConfig(data.type, data.config),
        config: data.config,
      };
      await apiUpdateSource(updatedSource);
      await reloadStorages();
      toast({
        title: "Storage updated",
        description: `Successfully updated "${data.name}".`,
      });
    } catch (error: unknown) {
      toast({
        title: "Failed to update storage",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setEditingStorage(null);
    }
  };

  const handleDeleteStorage = (id: string) => {
    const storage = storages.find((s) => s.id === id);
    void (async () => {
      try {
        await apiRemoveSource(id);
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
    const storage = storages.find((s) => s.id === id);
    void (async () => {
      toast({
        title: "Refreshing",
        description: `Refreshing ${storage?.name ?? "storage"}...`,
      });
      setStorageRefreshTick((prev) => ({
        ...prev,
        [id]: (prev[id] ?? 0) + 1,
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
        try {
          const text = (loadEvent.target?.result as string) ?? "";
          const parsed = JSON.parse(text);
          if (!Array.isArray(parsed)) {
            throw new Error("Expected a JSON array of storage configurations.");
          }

          const sources: Source[] = parsed.map((item, index) => {
            if (!item || typeof item !== "object") {
              throw new Error(`Invalid item at index ${index}.`);
            }

            // If it already looks like a Source
            if ("kind" in item && "root" in item) {
              const sourceItem = item as Partial<Source>;
              const id =
                typeof sourceItem.id === "string" && sourceItem.id.length > 0
                  ? sourceItem.id
                  : Math.random().toString(36).substring(2, 10);
              const name =
                typeof sourceItem.name === "string" && sourceItem.name.length > 0
                  ? sourceItem.name
                  : `Storage ${index + 1}`;
              const kind = (sourceItem.kind ?? "local") as SourceKind;
              const root = String(sourceItem.root ?? "");
              return { id, name, kind, root };
            }

            // Fallback: treat it like a StorageConfig shape
            if ("type" in item && "config" in item) {
              const storageItem = item as {
                id?: string;
                name?: string;
                type: StorageType;
                config: Record<string, string>;
              };
              const id =
                typeof storageItem.id === "string" && storageItem.id.length > 0
                  ? storageItem.id
                  : Math.random().toString(36).substring(2, 10);
              const name =
                typeof storageItem.name === "string" && storageItem.name.length > 0
                  ? storageItem.name
                  : `Storage ${index + 1}`;
              const type = storageItem.type ?? "local-fs";
              const config = storageItem.config ?? {};
              return {
                id,
                name,
                kind: mapStorageTypeToSourceKind(type),
                root: deriveRootFromConfig(type, config),
                config: config,
              };
            }

            throw new Error(`Unsupported storage format at index ${index}.`);
          });

          void (async () => {
            try {
              await apiReplaceSources(sources);
              await reloadStorages();
              toast({
                title: "Import successful",
                description: `Imported ${sources.length} storage configuration(s).`,
              });
            } catch (error: unknown) {
              toast({
                title: "Import failed",
                description: error instanceof Error ? error.message : String(error),
                variant: "destructive",
              });
            }
          })();
        } catch (error: unknown) {
          toast({
            title: "Import failed",
            description: error instanceof Error ? error.message : String(error),
            variant: "destructive",
          });
        }
      };

      reader.readAsText(file);
    };

    input.click();
  };

  const handleExportStorages = () => {
    const sources = storages.map(storageToSource);
    const dataStr = JSON.stringify(sources, null, 2);
    const blob = new Blob([dataStr], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "infimount-storages.json";
    link.click();
    URL.revokeObjectURL(url);

    toast({
      title: "Export successful",
      description: `Exported ${sources.length} storage configuration(s).`,
    });
  };

  const currentStorage = storages.find((s) => s.id === selectedStorage);

  const [isPreviewVisible, setIsPreviewVisible] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);

  useEffect(() => {
    const mql = window.matchMedia("(max-width: 767px)");
    const handle = () => {
      if (mql.matches) {
        setIsSidebarOpen(false);
      } else {
        setIsSidebarOpen(true);
      }
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

  const toggleSidebar = () => setIsSidebarOpen((prev) => !prev);
  const closeSidebar = () => setIsSidebarOpen(false);
  const handleSelectStorage = (id: string) => {
    setSelectedStorage(id);
    if (window.matchMedia("(max-width: 767px)").matches) {
      setIsSidebarOpen(false);
    }
  };

  return (
    <div className="flex h-screen w-full overflow-hidden rounded-[12px] bg-background border border-border/40">
      <PanelGroup direction="horizontal">
        {isSidebarOpen && (
          <>
            <Panel
              className="hidden md:block transition-all duration-200"
              defaultSize={20}
              minSize={15}
              maxSize={40}
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
                onExportStorages={handleExportStorages}
                isLoading={isStoragesLoading}
              />
            </Panel>
            <PanelResizeHandle className="hidden md:flex w-px flex-col items-center justify-center bg-transparent group/handle relative z-10">
              <div className="absolute inset-y-0 -left-1 -right-1 z-50 cursor-col-resize" />
              <div className="h-full w-[1px] bg-border/40 group-hover/handle:bg-primary/40 transition-colors" />
            </PanelResizeHandle>
          </>
        )}

        {/* Mobile Sidebar Overlay */}
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
            onExportStorages={handleExportStorages}
            isLoading={isStoragesLoading}
          />
        </div>

        {/* Main Content Area */}
        <Panel className="flex-1 overflow-hidden">
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
        </Panel>
      </PanelGroup>

      <AddStorageDialog
        open={isAddDialogOpen}
        onOpenChange={(open) => {
          setIsAddDialogOpen(open);
          if (!open) setEditingStorage(null);
        }}
        onAdd={handleAddStorage}
        onUpdate={handleUpdateStorage}
        initialStorage={editingStorage ?? undefined}
      />
    </div>
  );
};

export default Index;
