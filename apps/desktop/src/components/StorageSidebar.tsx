import { Plus, HardDrive, Cloud, Server, Folder, MoreVertical, Edit, Trash2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { StorageConfig } from "@/types/storage";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
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
}

const getStorageIcon = (type: string) => {
  switch (type) {
    case 'aws-s3':
      return Cloud;
    case 'azure-blob':
      return Cloud;
    case 'webdav':
      return Server;
    case 'local-fs':
      return HardDrive;
    default:
      return Folder;
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
}: StorageSidebarProps) {
  return (
    <div className="flex h-full flex-col border-r bg-sidebar">
      <div className="flex items-center justify-between border-b p-4">
        <h2 className="text-lg font-semibold text-sidebar-foreground">Storages</h2>
        <Button
          size="icon"
          variant="ghost"
          onClick={onAddStorage}
          className="h-8 w-8"
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>

      <ScrollArea className="flex-1 p-2">
        <div className="space-y-1">
          {storages.map((storage) => {
            const Icon = getStorageIcon(storage.type);
            return (
              <div
                key={storage.id}
                className={cn(
                  "w-full flex items-center gap-2 rounded-lg px-3 py-2.5 transition-colors group",
                  selectedStorage === storage.id
                    ? "bg-sidebar-accent text-sidebar-accent-foreground"
                    : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                )}
              >
                <button
                  onClick={() => onSelectStorage(storage.id)}
                  className="flex flex-1 items-center gap-3 text-left text-sm min-w-0"
                >
                  <Icon className="h-4 w-4 shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="truncate font-medium">{storage.name}</div>
                    <div className="text-xs text-muted-foreground truncate">
                      {storage.type.toUpperCase()}
                    </div>
                  </div>
                  {storage.connected && (
                    <div className="h-2 w-2 rounded-full bg-green-500 shrink-0" />
                  )}
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
                  <DropdownMenuContent align="end">
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
