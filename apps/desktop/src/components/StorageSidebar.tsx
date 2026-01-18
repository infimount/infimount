import { useState } from "react";
import type React from "react";
import {
  Plus,
  MoreVertical,
  Edit,
  Trash2,
  RefreshCw,
  GripVertical,
  Upload,
  Download,
} from "lucide-react";
import logo from "@/assets/openhsb-logo.png";
import s3Icon from "@/assets/amazon-s3.svg";
import azureIcon from "@/assets/azure-storage-blob.svg";
import gcsIcon from "@/assets/icons8-google-cloud.svg";
import webdavIcon from "@/assets/webdav.svg";
import folderIcon from "@/assets/folder.svg";
import folderNetworkIcon from "@/assets/folder-network.svg";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { StorageConfig } from "@/types/storage";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface StorageSidebarProps {
  storages: StorageConfig[];
  selectedStorage: string | null;
  onSelectStorage: (id: string) => void;
  onAddStorage: () => void;
  onEditStorage: (id: string) => void;
  onDeleteStorage: (id: string) => void;
  onRefreshStorage: (id: string) => void;
  onReorderStorages?: (startIndex: number, endIndex: number) => void;
  onImportStorages?: () => void;
  onExportStorages?: () => void;
}

const getStorageIcon = (type: string) => {
  switch (type) {
    case 'aws-s3':
      return s3Icon;
    case 'azure-blob':
      return azureIcon;
    case 'gcs':
      return gcsIcon;
    case 'webdav':
      return webdavIcon;
    case 'local-fs':
      return folderNetworkIcon;
    default:
      return folderIcon;
  }
};

export function StorageSidebar({
  storages,
  selectedStorage,
  onSelectStorage,
  onAddStorage,
  onEditStorage,
  onDeleteStorage,
  onRefreshStorage,
  onReorderStorages,
  onImportStorages,
  onExportStorages,
}: StorageSidebarProps) {
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);

  const handleDragStart = (event: React.DragEvent<HTMLDivElement>, index: number) => {
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
    }
    setDraggedIndex(index);
  };

  const handleDragOver = (e: React.DragEvent, index: number) => {
    if (!onReorderStorages) return;
    e.preventDefault();
    if (draggedIndex === null || draggedIndex === index) return;
    onReorderStorages(draggedIndex, index);
    setDraggedIndex(index);
  };

  const handleDragEnd = () => {
    setDraggedIndex(null);
  };

  return (
    <div className="flex h-full flex-col border-r bg-sidebar">
      <div className="flex items-center gap-3 border-b p-4">
        <img src={logo} alt="OpenHSB" className="h-10 w-10" />
        <div className="flex-1">
          <h2 className="text-lg text-sidebar-foreground">OpenHSB</h2>
          <p className="text-xs text-sidebar-foreground/70">Files Browser</p>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="icon" variant="ghost" className="h-8 w-8">
              <Plus className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md"
          >
            <DropdownMenuItem onClick={onAddStorage}>
              <Plus className="mr-2 h-4 w-4" />
              Add Storage
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            {onImportStorages && (
              <DropdownMenuItem onClick={onImportStorages}>
                <Upload className="mr-2 h-4 w-4" />
                Import Config
              </DropdownMenuItem>
            )}
            {onExportStorages && (
              <DropdownMenuItem onClick={onExportStorages}>
                <Download className="mr-2 h-4 w-4" />
                Export Config
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <ScrollArea className="flex-1 p-2">
        <div className="space-y-1">
          {storages.map((storage, index) => {
            const iconSrc = getStorageIcon(storage.type);
            return (
              <div
                key={storage.id}
                className={cn(
                  "w-full flex items-center gap-2 rounded-lg px-3 py-2.5 transition-colors group",
                  selectedStorage === storage.id
                    ? "bg-sidebar-accent text-sidebar-accent-foreground"
                    : "text-sidebar-foreground hover:bg-sidebar-accent/50",
                  onReorderStorages && "cursor-move",
                  draggedIndex === index && "opacity-50",
                )}
                draggable={Boolean(onReorderStorages)}
                onDragStart={(event) => handleDragStart(event, index)}
                onDragOver={(e) => handleDragOver(e, index)}
                onDragEnd={handleDragEnd}
              >
                {onReorderStorages && (
                  <GripVertical className="h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
                )}
                <button
                  onClick={() => onSelectStorage(storage.id)}
                  className="flex flex-1 items-center gap-3 text-left text-sm min-w-0"
                >
                  <img
                    src={iconSrc}
                    alt=""
                    aria-hidden="true"
                    className="h-5 w-5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="truncate font-normal">{storage.name}</div>
                    <div className="text-xs text-muted-foreground truncate">
                      {storage.type.toUpperCase()}
                    </div>
                  </div>
                </button>
                
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button 
                      variant="ghost" 
                      size="icon" 
                      className="h-6 w-6 shrink-0 opacity-0 group-hover:opacity-100"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <MoreVertical className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent
                    align="end"
                    className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md"
                  >
                    <DropdownMenuItem onClick={() => onRefreshStorage(storage.id)}>
                      <RefreshCw className="mr-2 h-4 w-4" />
                      Refresh
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => onEditStorage(storage.id)}>
                      <Edit className="mr-2 h-4 w-4" />
                      Edit
                    </DropdownMenuItem>
                    <DropdownMenuItem 
                      onClick={() => onDeleteStorage(storage.id)}
                      className="text-destructive"
                    >
                      <Trash2 className="mr-2 h-4 w-4" />
                      Delete
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            );
          })}
        </div>
      </ScrollArea>
    </div>
  );
}
