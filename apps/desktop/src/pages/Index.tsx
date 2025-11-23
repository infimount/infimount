import { useEffect, useState } from "react";
import { StorageSidebar } from "@/components/StorageSidebar";
import { AddStorageDialog } from "@/components/AddStorageDialog";
import { FileBrowser } from "@/components/FileBrowser";
import { StorageConfig, StorageType } from "@/types/storage";
import type { Source, SourceKind } from "@/types/source";
import { toast } from "@/hooks/use-toast";
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

const Index = () => {
  const [storages, setStorages] = useState<StorageConfig[]>([]);
  const [selectedStorage, setSelectedStorage] = useState<string | null>(null);
  const [isAddDialogOpen, setIsAddDialogOpen] = useState(false);
  const [editingStorage, setEditingStorage] = useState<StorageConfig | null>(null);

  const reloadStorages = async () => {
    try {
      const sources = await listSources();
      const mapped = sources.map(sourceToStorage);
      setStorages(mapped);
      if (!selectedStorage && mapped.length > 0) {
        setSelectedStorage(mapped[0].id);
      } else if (selectedStorage && !mapped.find((s) => s.id === selectedStorage)) {
        setSelectedStorage(mapped[0]?.id ?? null);
      }
    } catch (error: any) {
      toast({
        title: "Failed to load sources",
        description: error?.message || String(error),
        variant: "destructive",
      });
    }
  };

  useEffect(() => {
    void reloadStorages();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
    } catch (error: any) {
      toast({
        title: "Failed to add storage",
        description: error?.message || String(error),
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
    } catch (error: any) {
      toast({
        title: "Failed to update storage",
        description: error?.message || String(error),
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
      } catch (error: any) {
        toast({
          title: "Failed to delete storage",
          description: error?.message || String(error),
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
      await reloadStorages();
    })();
  };

  const handleReorderStorages = (startIndex: number, endIndex: number) => {
    if (startIndex === endIndex) return;
    setStorages((current) => {
      if (
        startIndex < 0 ||
        endIndex < 0 ||
        startIndex >= current.length ||
        endIndex >= current.length
      ) {
        return current;
      }
      const updated = [...current];
      const [moved] = updated.splice(startIndex, 1);
      updated.splice(endIndex, 0, moved);

      void (async () => {
        try {
          await apiReplaceSources(updated.map(storageToSource));
        } catch (error: any) {
          toast({
            title: "Failed to reorder storages",
            description: error?.message || String(error),
            variant: "destructive",
          });
          await reloadStorages();
        }
      })();

      return updated;
    });
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
            } catch (error: any) {
              toast({
                title: "Import failed",
                description: error?.message || String(error),
                variant: "destructive",
              });
            }
          })();
        } catch (error: any) {
          toast({
            title: "Import failed",
            description: error?.message || String(error),
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
    link.download = "openhsb-storages.json";
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
    if (isPreviewVisible && window.matchMedia("(max-width: 1024px)").matches) {
      setIsSidebarOpen(false);
    } else if (!isPreviewVisible) {
      // Optional: Auto-open when preview closes? 
      // The user didn't explicitly ask for this, but it's good UX.
      // Let's stick to the requested behavior: "visible in L also".
      // If we are on L, it never closed.
      // If we are on M, it closed. When preview closes, we probably want it back.
      setIsSidebarOpen(true);
    }
  }, [isPreviewVisible]);

  const toggleSidebar = () => setIsSidebarOpen((prev) => !prev);

  return (
    <div className="flex h-screen w-full overflow-hidden bg-background">
      <div
        className={`hidden md:resize-x overflow-hidden ${isSidebarOpen ? "md:block" : "md:hidden"}`}
        style={{ minWidth: "180px", maxWidth: "360px" }}
      >
        <StorageSidebar
          storages={storages}
          selectedStorage={selectedStorage}
          onSelectStorage={setSelectedStorage}
          onAddStorage={() => setIsAddDialogOpen(true)}
          onEditStorage={handleEditStorage}
          onDeleteStorage={handleDeleteStorage}
          onRefreshStorage={handleRefreshStorage}
          onReorderStorages={handleReorderStorages}
          onImportStorages={handleImportStorages}
          onExportStorages={handleExportStorages}
        />
      </div>

      <div className="flex-1 overflow-hidden">
        {currentStorage ? (
          <FileBrowser
            key={currentStorage.id}
            sourceId={currentStorage.id}
            storageName={currentStorage.name}
            onPreviewVisibilityChange={setIsPreviewVisible}
            onToggleSidebar={toggleSidebar}
          />
        ) : (
          <div className="flex h-full items-center justify-center">
            <p className="text-muted-foreground">Select a storage to view files</p>
          </div>
        )}
      </div>

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
