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
} from "@/lib/api";

const mapSourceKindToStorageType = (kind: SourceKind): StorageType => {
  switch (kind) {
    case "s3":
      return "aws-s3";
    case "azure_blob":
      return "azure-blob";
    case "webdav":
      return "webdav";
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
  return "";
};

const mapSourceToStorageConfig = (source: Source): StorageConfig => ({
  id: source.id,
  name: source.name,
  type: mapSourceKindToStorageType(source.kind),
  config: {},
  connected: true,
});

const Index = () => {
  const [storages, setStorages] = useState<StorageConfig[]>([]);
  const [selectedStorage, setSelectedStorage] = useState<string | null>(null);
  const [isAddDialogOpen, setIsAddDialogOpen] = useState(false);

  const reloadStorages = async () => {
    try {
      const sources = await listSources();
      const mapped = sources.map(mapSourceToStorageConfig);
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

  const handleAddStorage = (config: {
    name: string;
    type: StorageType;
    config: Record<string, string>;
  }) => {
    const source: Source = {
      id: Math.random().toString(36).substring(2, 10),
      name: config.name,
      kind: mapStorageTypeToSourceKind(config.type),
      root: deriveRootFromConfig(config.type, config.config),
    };

    void (async () => {
      try {
        await apiAddSource(source);
        await reloadStorages();
        toast({
          title: "Storage added",
          description: `${config.name} has been added successfully.`,
        });
      } catch (error: any) {
        toast({
          title: "Failed to add storage",
          description: error?.message || String(error),
          variant: "destructive",
        });
      }
    })();
  };

  const handleEditStorage = (id: string) => {
    const storage = storages.find(s => s.id === id);
    toast({
      title: "Edit storage",
      description: `Editing ${storage?.name}...`,
    });
    // Open edit dialog or navigate to edit page
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

  const currentStorage = storages.find((s) => s.id === selectedStorage);

  return (
    <div className="flex h-screen w-full overflow-hidden bg-background">
      <div className="hidden md:block md:w-64 md:shrink-0">
        <StorageSidebar
          storages={storages}
          selectedStorage={selectedStorage}
          onSelectStorage={setSelectedStorage}
          onAddStorage={() => setIsAddDialogOpen(true)}
          onEditStorage={handleEditStorage}
          onDeleteStorage={handleDeleteStorage}
          onRefreshStorage={handleRefreshStorage}
        />
      </div>

      <div className="flex-1 overflow-hidden">
        {currentStorage ? (
          <FileBrowser
            sourceId={currentStorage.id}
            storageName={currentStorage.name}
          />
        ) : (
          <div className="flex h-full items-center justify-center">
            <p className="text-muted-foreground">Select a storage to view files</p>
          </div>
        )}
      </div>

      <AddStorageDialog
        open={isAddDialogOpen}
        onOpenChange={setIsAddDialogOpen}
        onAdd={handleAddStorage}
      />
    </div>
  );
};

export default Index;
