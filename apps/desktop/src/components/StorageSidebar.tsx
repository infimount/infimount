import { useEffect, useRef, useState } from "react";
import {
  Menu,
  Plus,
  Edit,
  Trash2,
  RefreshCw,
  Upload,
  Download,
  Search,
  Sparkles,
} from "lucide-react";
import s3Icon from "@/assets/amazon-s3.svg";
import azureIcon from "@/assets/azure-storage-blob.svg";
import gcsIcon from "@/assets/icons8-google-cloud.svg";
import webdavIcon from "@/assets/webdav.svg";
import folderIcon from "@/assets/folder.svg";
import folderNetworkIcon from "@/assets/folder-network.svg";
import logo from "@/assets/icon-32x32.png";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useToast } from "@/hooks/use-toast";

interface StorageSidebarProps {
  storages: StorageConfig[];
  selectedStorage: string | null;
  onSelectStorage: (id: string) => void;
  onAddStorage: () => void;
  onEditStorage: (id: string) => void;
  onDeleteStorage: (id: string) => void;
  onRefreshStorage: (id: string) => void;
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
  onImportStorages,
  onExportStorages,
}: StorageSidebarProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchContainerRef = useRef<HTMLDivElement>(null);
  const { toast } = useToast();

  const handleCheckUpdate = () => {
    toast({
      title: "Checking for updates...",
      description: "You are on the latest version of Infimount (v0.0.1).",
    });
  };

  useEffect(() => {
    if (isSearchOpen) {
      searchInputRef.current?.focus();
    }
  }, [isSearchOpen]);

  useEffect(() => {
    if (!isSearchOpen) return;

    const handlePointerDown = (event: MouseEvent) => {
      if (!searchContainerRef.current) return;
      if (searchContainerRef.current.contains(event.target as Node)) return;
      setIsSearchOpen(false);
      setSearchQuery("");
    };

    document.addEventListener("mousedown", handlePointerDown);
    return () => document.removeEventListener("mousedown", handlePointerDown);
  }, [isSearchOpen]);

  const normalizedQuery = searchQuery.trim().toLowerCase();
  const visibleStorages = normalizedQuery
    ? storages.filter((storage) => storage.name.toLowerCase().includes(normalizedQuery))
    : storages;

  const handleToggleSearch = () => {
    if (isSearchOpen) {
      setIsSearchOpen(false);
      setSearchQuery("");
      return;
    }
    setIsSearchOpen(true);
  };

  return (
    <div className="flex h-full flex-col border-r bg-sidebar">
      <div className="relative border-b px-3 py-3" data-tauri-drag-region>
        <div className="grid grid-cols-[auto_1fr_auto] items-center gap-2">
          <div className="h-8 w-8" />
          <div
            className={cn(
              "text-center text-sm font-normal text-sidebar-foreground transition-opacity duration-200",
              isSearchOpen ? "opacity-0" : "opacity-100",
            )}
          >
            Storages
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8 hover:bg-sidebar-accent/30 hover:text-sidebar-foreground"
              >
                <Menu className="h-4 w-4" />
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
        <div
          ref={searchContainerRef}
          className="absolute inset-y-0 left-3 z-10 flex items-center transition-[width] duration-200"
          style={{ width: isSearchOpen ? "calc(100% - 1.5rem)" : "2rem" }}
        >
          <div
            className={cn(
              "flex h-8 w-full items-center overflow-hidden rounded-lg transition-all duration-200",
              isSearchOpen
                ? "border border-sidebar-border bg-background shadow-sm ring-1 ring-primary/10"
                : "border-transparent bg-transparent shadow-none",
            )}
          >
            <Button
              size="icon"
              variant="ghost"
              className="h-8 w-8 shrink-0 hover:bg-sidebar-accent/30 hover:text-sidebar-foreground"
              onClick={handleToggleSearch}
              title={isSearchOpen ? "Close search" : "Search storages"}
              aria-expanded={isSearchOpen}
            >
              <Search className="h-4 w-4" />
            </Button>
            <Input
              ref={searchInputRef}
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder="Search storages..."
              className={cn(
                "h-8 w-full border-0 bg-transparent px-0 py-0 text-sm transition-opacity duration-200 focus-visible:ring-0 focus-visible:ring-offset-0 focus-visible:border-transparent",
                isSearchOpen ? "opacity-100" : "pointer-events-none opacity-0",
              )}
              tabIndex={isSearchOpen ? 0 : -1}
            />
          </div>
        </div>
      </div>

      <ScrollArea className="flex-1 p-2">
        {visibleStorages.length === 0 ? (
          <div className="px-4 py-6 text-center text-xs text-muted-foreground">
            No storages found.
          </div>
        ) : (
          <div className="space-y-1">
            {visibleStorages.map((storage) => {
              const iconSrc = getStorageIcon(storage.type);
              return (
                <ContextMenu key={storage.id}>
                  <ContextMenuTrigger asChild>
                    <div
                      className={cn(
                        "w-full flex items-center gap-2 rounded-lg px-3 py-2.5 transition-colors group",
                        selectedStorage === storage.id
                          ? "bg-primary/20 text-sidebar-foreground font-medium"
                          : "text-sidebar-foreground hover:bg-black/5 dark:hover:bg-white/5",
                      )}
                      onContextMenu={() => onSelectStorage(storage.id)}
                    >
                      <button
                        onClick={() => onSelectStorage(storage.id)}
                        className="flex flex-1 items-center gap-2 text-left text-sm font-normal min-w-0"
                      >
                        <img
                          src={iconSrc}
                          alt=""
                          aria-hidden="true"
                          className="h-5 w-5 shrink-0"
                        />
                        <div className="flex-1 min-w-0">
                          <span className="block truncate text-[13px] font-normal leading-snug">
                            {storage.name}
                          </span>
                        </div>
                      </button>
                    </div>
                  </ContextMenuTrigger>
                  <ContextMenuContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                    <ContextMenuItem onClick={() => onRefreshStorage(storage.id)}>
                      <RefreshCw className="mr-2 h-4 w-4" />
                      Refresh
                    </ContextMenuItem>
                    <ContextMenuItem onClick={() => onEditStorage(storage.id)}>
                      <Edit className="mr-2 h-4 w-4" />
                      Edit
                    </ContextMenuItem>
                    <ContextMenuItem
                      onClick={() => onDeleteStorage(storage.id)}
                      className="text-foreground focus:text-foreground"
                    >
                      <Trash2 className="mr-2 h-4 w-4" />
                      Delete
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              );
            })}
          </div>
        )}
      </ScrollArea>

      <div className="border-t border-sidebar-border h-9 px-3 flex items-center justify-between shrink-0">
        <div className="flex items-center gap-2 overflow-hidden">
          <img src={logo} alt="" className="h-7 w-7 shrink-0 object-contain" />
          <div className="flex items-center gap-1.5 overflow-hidden">
            <span className="text-xs font-semibold text-sidebar-foreground truncate">Infimount</span>
            <span className="text-[10px] text-muted-foreground shrink-0">v0.0.1</span>
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 text-muted-foreground hover:text-foreground"
          onClick={handleCheckUpdate}
          title="Check for updates"
        >
          <Sparkles className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
