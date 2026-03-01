import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { check as checkUpdater } from "@tauri-apps/plugin-updater";
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
import gcsIcon from "@/assets/google-cloud.svg";
import webdavIcon from "@/assets/webdav.svg";
import folderIcon from "@/assets/folder.svg";
import folderNetworkIcon from "@/assets/folder-network.svg";
import logo from "@/assets/icon-128x128.png";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { StorageConfig } from "@/types/storage";
import { transferEntries, TauriApiError } from "@/lib/api";
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
  isLoading?: boolean;
}

const RELEASES_PAGE_URL = "https://github.com/infimount/infimount/releases/latest";
const normalizeVersion = (value: string): string => value.trim().replace(/^v/i, "");

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
  isLoading = false,
}: StorageSidebarProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchContainerRef = useRef<HTMLDivElement>(null);
  const [dragTargetId, setDragTargetId] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null | undefined>(undefined);
  const [isCheckingUpdates, setIsCheckingUpdates] = useState(false);
  const hasAutoCheckedUpdatesRef = useRef(false);
  const [dropConflict, setDropConflict] = useState<{
    fromSourceId: string;
    toSourceId: string;
    paths: string[];
    operation: "copy" | "move";
  } | null>(null);
  const { toast } = useToast();

  const INTERNAL_TRANSFER_MIME = "application/x-infimount-transfer";

  const parseInternalTransfer = (dt: DataTransfer) => {
    const raw =
      dt.getData(INTERNAL_TRANSFER_MIME)
      || dt.getData("text/plain");
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw) as {
        kind?: string;
        fromSourceId?: string;
        paths?: unknown;
        operation?: string;
      };

      if (parsed?.kind !== "infimount-transfer") return null;
      if (!parsed.fromSourceId || typeof parsed.fromSourceId !== "string") return null;
      if (!Array.isArray(parsed.paths)) return null;
      const paths = parsed.paths.filter((p) => typeof p === "string") as string[];
      if (paths.length === 0) return null;

      const operation = parsed.operation === "move" ? "move" : "copy";
      return { fromSourceId: parsed.fromSourceId, paths, operation } as const;
    } catch {
      return null;
    }
  };

  const isExternalFileDrag = (dt: DataTransfer) => {
    const types = Array.from(dt.types ?? []);
    if (types.includes("Files")) return true;
    if (dt.files && dt.files.length > 0) return true;

    const items = Array.from(dt.items ?? []);
    return items.some((item) => item.kind === "file");
  };

  const isLikelyInternalTransferDrag = (dt: DataTransfer) => {
    const types = Array.from(dt.types ?? []);

    // Prefer explicit type when available.
    if (types.includes(INTERNAL_TRANSFER_MIME)) return true;

    // WebKit can hide custom types; fall back to plain text as long as it's not an OS file drag.
    if (isExternalFileDrag(dt)) return false;
    return types.includes("text/plain") || types.includes("Text");
  };

  const handleInternalDrop = async (
    toSourceId: string,
    payload: { fromSourceId: string; paths: string[]; operation: "copy" | "move" },
    conflictPolicy: "fail" | "overwrite" | "skip" = "fail",
  ) => {
    try {
      await transferEntries(
        payload.fromSourceId,
        toSourceId,
        payload.paths,
        "/",
        payload.operation,
        conflictPolicy,
      );
      toast({
        title: payload.operation === "copy" ? "Copied" : "Moved",
        description: `${payload.paths.length} item${payload.paths.length === 1 ? "" : "s"} ${
          payload.operation === "copy" ? "copied" : "moved"
        }.`,
      });
    } catch (error) {
      if (error instanceof TauriApiError) {
        if (error.code === "ALREADY_EXISTS" && conflictPolicy === "fail") {
          setDropConflict({
            fromSourceId: payload.fromSourceId,
            toSourceId,
            paths: payload.paths,
            operation: payload.operation,
          });
          return;
        }
        toast({
          title: "Transfer failed",
          description: error.message,
          variant: "destructive",
        });
      } else {
        toast({
          title: "Transfer failed",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      }
    }
  };

  const checkForUpdates = useCallback(
    async (silent = false) => {
      if (isCheckingUpdates) return;

      setIsCheckingUpdates(true);
      if (!silent) {
        toast({
          title: "Checking for updates...",
          description: "Looking for an in-app update package.",
        });
      }

      try {
        const update = await checkUpdater();
        if (!update) {
          if (!silent) {
            toast({
              title: "You're up to date",
              description: `Infimount v${appVersion ? normalizeVersion(appVersion) : "current"} is the latest available build.`,
            });
          }
          return;
        }

        if (silent) {
          toast({
            title: `Update available: v${update.version}`,
            description: "Click the sparkle icon to download and install.",
          });
          return;
        }

        const shouldInstall = window.confirm(
          `Update v${update.version} is available (current: v${update.currentVersion}).\n\nDownload and install now?`,
        );
        if (!shouldInstall) {
          return;
        }

        toast({
          title: `Downloading update v${update.version}...`,
          description: "Please wait while Infimount installs the update package.",
        });
        await update.downloadAndInstall();
        setAppVersion(normalizeVersion(update.version));
        toast({
          title: "Update installed",
          description: "Restart Infimount to apply the new version.",
        });
      } catch (error) {
        if (!silent) {
          toast({
            title: "Update check failed",
            description:
              error instanceof Error
                ? `${error.message}. You can download manually: ${RELEASES_PAGE_URL}`
                : `Unable to check updates. Visit ${RELEASES_PAGE_URL}`,
            variant: "destructive",
          });
        }
      } finally {
        setIsCheckingUpdates(false);
      }
    },
    [appVersion, isCheckingUpdates, toast],
  );

  const handleCheckUpdate = () => {
    void checkForUpdates(false);
  };

  useEffect(() => {
    let active = true;
    void getAppVersion()
      .then((version) => {
        if (!active) return;
        setAppVersion(normalizeVersion(version));
      })
      .catch(() => {
        if (!active) return;
        setAppVersion(null);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (hasAutoCheckedUpdatesRef.current) return;
    if (appVersion === undefined) return;

    hasAutoCheckedUpdatesRef.current = true;
    void checkForUpdates(true);
  }, [appVersion, checkForUpdates]);

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
    <div className="flex h-full min-w-0 flex-col overflow-hidden border-r bg-sidebar">
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
        {isLoading && storages.length > 0 && (
          <div className="px-3 py-2 text-xs text-muted-foreground">
            Loading…
          </div>
        )}
        {visibleStorages.length === 0 ? (
          isLoading ? (
            <div className="px-4 py-6 text-center text-xs text-muted-foreground">
              Loading storages…
            </div>
          ) : (
            <div className="px-4 py-6 text-center text-xs text-muted-foreground">
              No storages found.
            </div>
          )
        ) : (
          <div className="space-y-1">
            {visibleStorages.map((storage) => {
              const iconSrc = getStorageIcon(storage.type);
              const isDragTarget = dragTargetId === storage.id;
              return (
                <ContextMenu key={storage.id}>
                  <ContextMenuTrigger asChild>
                    <div
                      className={cn(
                        "group flex w-full items-center gap-2 overflow-hidden rounded-lg px-3 py-2.5 transition-colors",
                        selectedStorage === storage.id
                          ? "bg-primary/20 text-sidebar-foreground font-medium"
                          : "text-sidebar-foreground hover:bg-black/5 dark:hover:bg-white/5",
                        isDragTarget && selectedStorage !== storage.id && "bg-primary/10 ring-1 ring-primary/30",
                      )}
                      onContextMenu={() => onSelectStorage(storage.id)}
                      onDragOver={(event) => {
                        if (!isLikelyInternalTransferDrag(event.dataTransfer)) return;
                        event.preventDefault();
                        event.stopPropagation();
                        event.dataTransfer.dropEffect = "copy";
                        setDragTargetId(storage.id);
                      }}
                      onDragLeave={() => {
                        setDragTargetId((prev) => (prev === storage.id ? null : prev));
                      }}
                      onDrop={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        setDragTargetId(null);
                        const payload = parseInternalTransfer(event.dataTransfer);
                        if (!payload) return;
                        void handleInternalDrop(storage.id, payload);
                      }}
                    >
                      <button
                        onClick={() => onSelectStorage(storage.id)}
                        className="flex w-full flex-1 items-center gap-2 overflow-hidden text-left text-sm font-normal min-w-0"
                      >
                        <img
                          src={iconSrc}
                          alt=""
                          aria-hidden="true"
                          draggable={false}
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
          <img src={logo} alt="" className="h-7 w-7 shrink-0 object-contain" draggable={false} />
          <div className="flex items-center gap-1.5 overflow-hidden">
            <span className="text-xs font-semibold text-sidebar-foreground truncate">Infimount</span>
            <span className="text-[10px] text-muted-foreground shrink-0">
              {appVersion ? `v${appVersion}` : "v-"}
            </span>
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 text-muted-foreground hover:text-foreground"
          onClick={handleCheckUpdate}
          title={isCheckingUpdates ? "Checking updates..." : "Check for updates"}
          disabled={isCheckingUpdates}
        >
          <Sparkles className="h-4 w-4" />
        </Button>
      </div>

      <AlertDialog
        open={!!dropConflict}
        onOpenChange={(open) => {
          if (!open) setDropConflict(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Item already exists</AlertDialogTitle>
            <AlertDialogDescription>
              One or more items with the same name already exist in this destination. Do you want to overwrite them or discard this transfer?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-muted text-foreground hover:bg-muted/80"
              onClick={() => {
                if (!dropConflict) return;
                void (async () => {
                  await handleInternalDrop(
                    dropConflict.toSourceId,
                    {
                      fromSourceId: dropConflict.fromSourceId,
                      paths: dropConflict.paths,
                      operation: dropConflict.operation,
                    },
                    "skip",
                  );
                  setDropConflict(null);
                })();
              }}
            >
              Discard
            </AlertDialogAction>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                if (!dropConflict) return;
                void (async () => {
                  await handleInternalDrop(
                    dropConflict.toSourceId,
                    {
                      fromSourceId: dropConflict.fromSourceId,
                      paths: dropConflict.paths,
                      operation: dropConflict.operation,
                    },
                    "overwrite",
                  );
                  setDropConflict(null);
                })();
              }}
            >
              Overwrite
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
