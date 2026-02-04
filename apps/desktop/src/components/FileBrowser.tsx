import { useEffect, useState, useRef } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import {
  Search,
  LayoutGrid,
  LayoutList,
  ChevronLeft,
  ChevronRight,
  Upload,
  PanelLeft,
  PanelRight,
  Palette,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { WindowControls } from "./WindowControls";
import { FileGrid } from "./FileGrid";
import { FileTable } from "./FileTable";
import { UploadZone, type UploadFileLike, type UploadZoneRef } from "./UploadZone";
import { FilePreviewPanel } from "./FilePreviewPanel";
import { FileItem } from "@/types/storage";
import {
  Entry,
  listEntries,
  readFile,
  writeFile,
  deletePath,
  transferEntries,
  TauriApiError,
} from "@/lib/api";
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
import { toast } from "@/hooks/use-toast";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DEFAULT_ICON_THEME,
  ICON_THEME_LABELS,
  ICON_THEME_OPTIONS,
  type IconTheme,
  useIconTheme,
} from "@/hooks/use-icon-theme";
import { useFileClipboard } from "@/hooks/use-file-clipboard";

// Helper to extract file-like objects (including from dropped folders, where supported)
async function collectFilesFromDataTransfer(
  dt: DataTransfer
): Promise<UploadFileLike[]> {
  const items = dt.items;
  const files: UploadFileLike[] = [];

  if (!items || items.length === 0) {
    const fallback = Array.from(dt.files ?? []);
    return fallback.map((f) => ({
      name: f.name,
      arrayBuffer: () => f.arrayBuffer(),
    }));
  }

  // Non-standard folder support via webkitGetAsEntry (where available).
  const walkEntry = async (
    entry: any,
    parentPath: string,
  ): Promise<UploadFileLike[]> => {
    if (!entry) return [];

    if (entry.isFile) {
      const file: File = await new Promise((resolve, reject) => {
        entry.file(
          (f: File) => resolve(f),
          (err: unknown) => reject(err)
        );
      });
      const relativeName = parentPath ? `${parentPath}/${file.name}` : file.name;
      return [
        {
          name: relativeName,
          arrayBuffer: () => file.arrayBuffer(),
        },
      ];
    }

    if (entry.isDirectory) {
      const reader = entry.createReader();
      const entries: any[] = [];

      await new Promise<void>((resolve, reject) => {
        const readBatch = () => {
          reader.readEntries(
            (batch: any[]) => {
              if (!batch.length) {
                resolve();
                return;
              }
              entries.push(...batch);
              readBatch();
            },
            (err: unknown) => reject(err)
          );
        };
        readBatch();
      });

      const nestedFiles: UploadFileLike[] = [];
      for (const child of entries) {
        const childDirPath = parentPath
          ? `${parentPath}/${entry.name}`
          : entry.name;
        const childFiles = await walkEntry(child, childDirPath);
        nestedFiles.push(...childFiles);
      }
      return nestedFiles;
    }

    return [];
  };

  const entryPromises: Promise<UploadFileLike[]>[] = [];

  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    if (item.kind !== "file") continue;

    const anyItem = item as any;
    if (typeof anyItem.webkitGetAsEntry === "function") {
      const entry = anyItem.webkitGetAsEntry();
      if (entry) {
        entryPromises.push(walkEntry(entry, ""));
        continue;
      }
    }

    const file = item.getAsFile();
    if (file) {
      files.push({
        name: file.name,
        arrayBuffer: () => file.arrayBuffer(),
      });
    }
  }

  if (entryPromises.length > 0) {
    const nested = await Promise.all(entryPromises);
    nested.forEach((group) => files.push(...group));
  }

  // Fallback if nothing collected via items.
  if (files.length === 0) {
    const fallback = Array.from(dt.files ?? []);
    return fallback.map((f) => ({
      name: f.name,
      arrayBuffer: () => f.arrayBuffer(),
    }));
  }

  return files;
}

interface FileBrowserProps {
  sourceId: string;
  storageName: string;
  onPreviewVisibilityChange?: (visible: boolean) => void;
  onToggleSidebar?: () => void;
  isSidebarOpen?: boolean;
}

interface LoadError {
  title: string;
  detail?: string;
}

export function FileBrowser({
  sourceId,
  storageName,
  onPreviewVisibilityChange,
  onToggleSidebar,
  isSidebarOpen,
}: FileBrowserProps) {
  const [viewMode, setViewMode] = useState<"grid" | "table">("grid");
  const [searchQuery, setSearchQuery] = useState('');
  const [currentPath, setCurrentPath] = useState<string>("/");
  const [allFiles, setAllFiles] = useState<FileItem[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [history, setHistory] = useState<string[]>(["/"]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<LoadError | null>(null);

  type SortField = "name" | "type" | "modified" | "size";
  type SortDirection = "asc" | "desc";

  const [sortField, setSortField] = useState<SortField>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const [previewFile, setPreviewFile] = useState<FileItem | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [pasteConflict, setPasteConflict] = useState<{
    fromSourceId: string;
    toSourceId: string;
    paths: string[];
    targetDir: string;
    operation: "copy" | "move";
  } | null>(null);
  const [editTargetId, setEditTargetId] = useState<string | null>(null);
  const { theme: iconTheme, setTheme: setIconTheme } = useIconTheme();
  const { clipboard, setClipboard, clearClipboard } = useFileClipboard();
  const [isEditingPath, setIsEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");

  const describeLoadError = (err: TauriApiError): LoadError => {
    const shortMessage = (err.message || "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)[0];

    switch (err.code) {
      case "NOT_FOUND":
        return {
          title: "Folder not found",
          detail: "The requested path does not exist on this storage.",
        };
      case "PERMISSION_DENIED":
        return {
          title: "Access denied",
          detail: "You don't have permission to view this location.",
        };
      case "CONFIG_ERROR":
        return {
          title: "Can't connect to this storage",
          detail: "Check the credentials, endpoint URL, or bucket/container settings.",
        };
      case "IO_ERROR":
        return {
          title: "Network issue",
          detail: "Unable to reach the storage service. Verify network/VPN settings.",
        };
      case "TIMEOUT":
        return {
          title: "Timed out",
          detail: "The request took too long. Please check your connection and retry.",
        };
      default:
        return {
          title: "Could not connect to this storage",
          detail: shortMessage || err.message,
        };
    }
  };

  useEffect(() => {
    onPreviewVisibilityChange?.(!!previewFile);
  }, [previewFile, onPreviewVisibilityChange]);

  const filteredFiles = allFiles.filter((file) =>
    file.name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  const mapEntryToFileItem = (entry: Entry): FileItem => ({
    id: entry.path,
    name: entry.name,
    type: entry.is_dir ? "folder" : "file",
    size: entry.is_dir ? undefined : entry.size,
    modified: entry.modified_at ? new Date(entry.modified_at) : null,
    owner: undefined,
    extension: !entry.is_dir ? entry.name.split(".").pop() : undefined,
  });

  const loadFiles = async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const entries = await listEntries(sourceId, path);
      const filtered = entries.filter(
        (e) => e.path !== path && e.path !== "" && e.name !== ".",
      );
      setAllFiles(filtered.map(mapEntryToFileItem));
      setSelectedFiles(new Set());
    } catch (err) {
      if (err instanceof TauriApiError) {
        // If the root path is empty/not found, treat it as an empty folder instead of an error.
        if (err.code === "NOT_FOUND" && (path === "/" || path === "")) {
          setAllFiles([]);
          setError(null);
          setSelectedFiles(new Set());
          setLoading(false);
          return;
        }
        setError(describeLoadError(err));
      } else {
        setError({
          title: "Failed to load files",
          detail: err instanceof Error ? err.message : String(err),
        });
      }
      setAllFiles([]); // Clear files on error to prevent showing stale data
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    setCurrentPath("/");
    setAllFiles([]); // Clear files when switching sources
    setSelectedFiles(new Set());
    setHistory(["/"]);
    setHistoryIndex(0);
    setPreviewFile(null);
    setEditTargetId(null);
  }, [sourceId]);

  useEffect(() => {
    void loadFiles(currentPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPath, sourceId]);

  const handleNavigate = (path: string, options?: { fromHistory?: boolean }) => {
    const normalized = path || "/";
    setSearchQuery("");
    setSelectedFiles(new Set());
    if (options?.fromHistory) {
      setCurrentPath(normalized);
      return;
    }

    setHistory((previous) => {
      const trimmed = previous.slice(0, historyIndex + 1);
      if (trimmed[trimmed.length - 1] === normalized) {
        return trimmed;
      }
      const next = [...trimmed, normalized];
      setHistoryIndex(next.length - 1);
      return next;
    });
    setCurrentPath(normalized);
  };

  const canGoBack = historyIndex > 0;
  const canGoForward = historyIndex < history.length - 1;

  const goBack = () => {
    if (!canGoBack) return;
    const newIndex = historyIndex - 1;
    setHistoryIndex(newIndex);
    const target = history[newIndex] || "/";
    handleNavigate(target, { fromHistory: true });
  };

  const goForward = () => {
    if (!canGoForward) return;
    const newIndex = historyIndex + 1;
    setHistoryIndex(newIndex);
    const target = history[newIndex] || "/";
    handleNavigate(target, { fromHistory: true });
  };

  const handleSelectFile = (fileId: string, options?: { toggle?: boolean }) => {
    if (options?.toggle) {
      setSelectedFiles((prev) => {
        const newSet = new Set(prev);
        if (newSet.has(fileId)) {
          newSet.delete(fileId);
        } else {
          newSet.add(fileId);
        }
        return newSet;
      });
      return;
    }
    setSelectedFiles(new Set([fileId]));
  };

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "a") {
        return;
      }
      const active = document.activeElement as HTMLElement | null;
      if (
        active &&
        (active.tagName === "INPUT" ||
          active.tagName === "TEXTAREA" ||
          active.isContentEditable)
      ) {
        return;
      }
      event.preventDefault();
      setSelectedFiles(new Set(filteredFiles.map((file) => file.id)));
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [filteredFiles]);

  const setClipboardFromSelection = (operation: "copy" | "move") => {
    if (selectedFiles.size === 0) return;
    setClipboard({
      operation,
      sourceId,
      paths: Array.from(selectedFiles),
    });
  };

  const pasteInto = async (targetDir?: string) => {
    if (!clipboard || clipboard.paths.length === 0) {
      return;
    }

    const destinationDir = targetDir ?? currentPath;
    try {
      await transferEntries(
        clipboard.sourceId,
        sourceId,
        clipboard.paths,
        destinationDir,
        clipboard.operation,
        "fail",
      );
      await loadFiles(currentPath);
      toast({
        title: clipboard.operation === "copy" ? "Copied" : "Moved",
        description: `${clipboard.paths.length} item${clipboard.paths.length === 1 ? "" : "s"} ${
          clipboard.operation === "copy" ? "copied" : "moved"
        }.`,
      });
      if (clipboard.operation === "move") {
        clearClipboard();
      }
    } catch (err) {
      if (err instanceof TauriApiError) {
        if (err.code === "ALREADY_EXISTS" && clipboard.operation === "copy") {
          setPasteConflict({
            fromSourceId: clipboard.sourceId,
            toSourceId: sourceId,
            paths: clipboard.paths,
            targetDir: destinationDir,
            operation: clipboard.operation,
          });
          return;
        }
        toast({
          title: clipboard.operation === "copy" ? "Copy failed" : "Move failed",
          description: err.message,
          variant: "destructive",
        });
      } else {
        toast({
          title: clipboard.operation === "copy" ? "Copy failed" : "Move failed",
          description: err instanceof Error ? err.message : String(err),
          variant: "destructive",
        });
      }
    }
  };

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) {
        return;
      }

      const active = document.activeElement as HTMLElement | null;
      if (
        active &&
        (active.tagName === "INPUT" ||
          active.tagName === "TEXTAREA" ||
          active.isContentEditable)
      ) {
        return;
      }

      const key = event.key.toLowerCase();

      if (key === "c") {
        if (selectedFiles.size === 0) return;
        event.preventDefault();
        setClipboardFromSelection("copy");
      } else if (key === "x") {
        if (selectedFiles.size === 0) return;
        event.preventDefault();
        setClipboardFromSelection("move");
      } else if (key === "v") {
        if (!clipboard) return;
        event.preventDefault();
        void pasteInto();
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [clipboard, currentPath, selectedFiles, setClipboard, sourceId]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (
        (event.key !== "Delete" && event.key !== "Backspace") ||
        selectedFiles.size === 0
      ) {
        return;
      }
      const active = document.activeElement as HTMLElement | null;
      if (
        active &&
        (active.tagName === "INPUT" ||
          active.tagName === "TEXTAREA" ||
          active.isContentEditable)
      ) {
        return;
      }
      event.preventDefault();
      setShowDeleteConfirm(true);
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectedFiles]);

  const downloadOne = async (file: FileItem) => {
    if (file.type !== "file") return;
    try {
      const data = await readFile(sourceId, file.id);
      const arrayBuffer = data.buffer.slice(
        data.byteOffset,
        data.byteOffset + data.byteLength,
      ) as ArrayBuffer;
      const blob = new Blob([arrayBuffer], {
        type: "application/octet-stream",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = file.name;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      toast({
        title: "Download started",
        description: `Downloading "${file.name}"`,
      });
    } catch (error: any) {
      toast({
        title: "Download failed",
        description: error?.message || String(error),
        variant: "destructive",
      });
    }
  };

  const deleteOne = async (file: FileItem) => {
    try {
      await deletePath(sourceId, file.id);
    } catch (error: unknown) {
      toast({
        title: "Delete failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    }
  };

  const handleBulkDelete = async () => {
    const toDelete = filteredFiles.filter((f) => selectedFiles.has(f.id));
    for (const file of toDelete) {
      await deleteOne(file);
    }
    await loadFiles(currentPath);
    toast({
      title: "Items deleted",
      description: `${toDelete.length} item(s) removed.`,
      variant: "default",
    });
  };

  const handleOpenFile = (file: FileItem) => {
    if (file.type === "folder") {
      handleNavigate(file.id);
    } else {
      setEditTargetId(null);
      setPreviewFile(file);
    }
  };

  const handleEditFile = (file: FileItem) => {
    if (file.type === "folder") return;
    setPreviewFile(file);
    setEditTargetId(file.id);
  };

  const handleDownloadFile = (file: FileItem) => {
    void downloadOne(file);
  };

  const handleUpload = (files: UploadFileLike[]) => {
    void (async () => {
      if (!files.length) return;
      let successCount = 0;
      for (const file of files) {
        try {
          const buffer = await file.arrayBuffer();
          const basePath = currentPath === "/" ? "" : currentPath.replace(/\/$/, "");
          const targetPath = `${basePath}/${file.name}`;
          await writeFile(sourceId, targetPath, new Uint8Array(buffer));
          successCount += 1;
        } catch (error: unknown) {
          toast({
            title: "Upload failed",
            description: error instanceof Error ? error.message : String(error),
            variant: "destructive",
          });
        }
      }
      await loadFiles(currentPath);
      if (successCount > 0) {
        toast({
          title: "Upload complete",
          description: `${successCount} file${successCount > 1 ? "s" : ""} uploaded successfully.`,
        });
      }
    })();
  };

  const toggleSort = (field: SortField) => {
    if (sortField === field) {
      if (sortDirection === "asc") {
        setSortDirection("desc");
      } else if (sortDirection === "desc") {
        // Reset back to default: name asc
        setSortField("name");
        setSortDirection("asc");
      }
      return;
    }
    setSortField(field);
    setSortDirection(field === "modified" || field === "size" ? "desc" : "asc");
  };

  const sortedFiles = [...filteredFiles].sort((a, b) => {
    const dir = sortDirection === "asc" ? 1 : -1;
    switch (sortField) {
      case "name":
        return a.name.localeCompare(b.name) * dir;
      case "type": {
        const ta = a.type === "folder" ? "0" : (a.extension || "z");
        const tb = b.type === "folder" ? "0" : (b.extension || "z");
        return ta.localeCompare(tb) * dir;
      }
      case "size": {
        const sa = a.size ?? 0;
        const sb = b.size ?? 0;
        return (sa - sb) * dir;
      }
      case "modified": {
        const ma = a.modified ? a.modified.getTime() : 0;
        const mb = b.modified ? b.modified.getTime() : 0;
        return (ma - mb) * dir;
      }
      default:
        return 0;
    }
  });

  const getBreadcrumbs = () => {
    const parts = currentPath.split("/").filter(Boolean);
    const items: { name: string; path: string }[] = [{ name: storageName, path: "/" }];
    let acc = "";
    for (const part of parts) {
      acc += "/" + part;
      items.push({ name: part, path: acc });
    }
    return items;
  };

  const breadcrumbs = getBreadcrumbs();
  const currentLabel =
    breadcrumbs[breadcrumbs.length - 1]?.name ?? storageName;

  const [isDragging, setIsDragging] = useState(false);
  const uploadZoneRef = useRef<UploadZoneRef | null>(null);

  const isFileDrag = (event: React.DragEvent) => {
    const types = Array.from(event.dataTransfer?.types ?? []);
    return types.includes("Files") || types.includes("text/uri-list");
  };

  return (
    <>
      <div
        className="relative flex h-full bg-background"
        onDragOver={(event: React.DragEvent<HTMLDivElement>) => {
          if (!isFileDrag(event)) return;
          event.preventDefault();
          event.stopPropagation();
          event.dataTransfer.dropEffect = "copy";
          setIsDragging(true);
        }}
        onDragLeave={(event: React.DragEvent<HTMLDivElement>) => {
          event.preventDefault();
          event.stopPropagation();
          setIsDragging(false);
        }}
        onDrop={(event: React.DragEvent<HTMLDivElement>) => {
          if (!isFileDrag(event)) return;
          event.preventDefault();
          event.stopPropagation();
          setIsDragging(false);
          if (!event.dataTransfer || !uploadZoneRef.current) return;

          void (async () => {
            try {
              const files = await collectFilesFromDataTransfer(event.dataTransfer);
              if (files.length) {
                uploadZoneRef.current?.handleFiles(files);
              }
            } catch (err: unknown) {
              // If folder support is not available, fall back gracefully.
              toast({
                title: "Upload failed",
                description:
                  err instanceof Error
                    ? err.message
                    : "Could not read some dropped items. Try dropping files only or use the file picker.",
                variant: "destructive",
              });
            }
          })();
        }}
      >
        <div className="flex flex-1 flex-col">
          {/* Header with navigation */}
          <div className="border-b bg-muted/30" data-tauri-drag-region>
            <div className="flex items-center gap-2 px-4 py-3" data-tauri-drag-region>
              <div className="flex items-center gap-1 tauri-no-drag">
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-8 w-8 mr-1 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                  onClick={onToggleSidebar}
                  title={isSidebarOpen ? "Hide Storage Sidebar" : "Show Storage Sidebar"}
                >
                  {isSidebarOpen ? (
                    <PanelRight className="h-4 w-4" />
                  ) : (
                    <PanelLeft className="h-4 w-4" />
                  )}
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                  onClick={goBack}
                  disabled={!canGoBack}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                  onClick={goForward}
                  disabled={!canGoForward}
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>

              <div className="flex min-w-0 flex-1 items-center gap-2" data-tauri-drag-region>
                <span className="truncate text-sm font-medium select-none pointer-events-none">
                  {currentLabel}
                </span>
              </div>

              <div className="flex items-center gap-2 tauri-no-drag">
                <div className="relative w-48">
                  <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    placeholder="Search..."
                    value={searchQuery}
                    onChange={(event) => setSearchQuery(event.target.value)}
                    className="h-8 bg-background pl-9 border-border focus-visible:ring-1 focus-visible:ring-primary/20 focus-visible:ring-offset-0 focus-visible:border-border shadow-sm"
                  />
                </div>

                <label htmlFor="file-upload">
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                    asChild
                  >
                    <span>
                      <Upload className="h-4 w-4" />
                    </span>
                  </Button>
                </label>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                      title={`Icon theme: ${ICON_THEME_LABELS[iconTheme] ?? ICON_THEME_LABELS[DEFAULT_ICON_THEME]}`}
                    >
                      <Palette className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="min-w-[160px]">
                    <DropdownMenuLabel className="font-normal">Icon Theme</DropdownMenuLabel>
                    <DropdownMenuSeparator />
                    <DropdownMenuRadioGroup
                      value={iconTheme}
                      onValueChange={(value) => setIconTheme(value as IconTheme)}
                    >
                      {ICON_THEME_OPTIONS.map((theme) => (
                        <DropdownMenuRadioItem key={theme} value={theme}>
                          {ICON_THEME_LABELS[theme]}
                        </DropdownMenuRadioItem>
                      ))}
                    </DropdownMenuRadioGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() =>
                    setViewMode((current) => (current === "grid" ? "table" : "grid"))
                  }
                  className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                  title={viewMode === "grid" ? "Switch to list view" : "Switch to grid view"}
                >
                  {viewMode === "grid" ? (
                    <LayoutList className="h-4 w-4" />
                  ) : (
                    <LayoutGrid className="h-4 w-4" />
                  )}
                </Button>
                {previewFile && (
                  <Button
                    size="icon"
                    variant="ghost"
                    onClick={() => setPreviewFile(null)}
                    className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                    title="Close Preview"
                  >
                    <PanelRight className="h-4 w-4" />
                  </Button>
                )}
                <div className="ml-2 pl-2 border-l border-border/50">
                  <WindowControls />
                </div>
              </div>
            </div>
          </div>

          {/* Error */}
          {error && (
            <div className="border-b bg-gradient-to-r from-destructive/10 via-destructive/5 to-transparent px-6 py-3 text-sm text-destructive">
              <div className="flex items-start gap-3">
                <div className="mt-1 flex h-6 w-6 items-center justify-center rounded-full bg-destructive/20 text-destructive">
                  !
                </div>
                <div className="space-y-1">
                  <p className="font-semibold leading-tight">{error.title}</p>
                  {error.detail && (
                    <p className="text-xs leading-relaxed text-destructive/80">
                      {error.detail}
                    </p>
                  )}
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 border-destructive/40 text-destructive hover:bg-destructive/10"
                      onClick={() => {
                        void loadFiles(currentPath);
                      }}
                    >
                      Try again
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Panel Group for Content & Preview */}
          <PanelGroup direction="horizontal" className="flex-1 overflow-hidden">
            <Panel minSize={30} defaultSize={previewFile ? 70 : 100}>
                <div className="flex h-full flex-col overflow-hidden relative">
                <ContextMenu>
                  <ContextMenuTrigger asChild>
                    <div className="flex-1 overflow-hidden">
                      {loading ? (
                        <div className="flex h-full items-center justify-center">
                          <div className="flex flex-col items-center gap-2">
                            <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                            <span className="text-muted-foreground">Loading files...</span>
                          </div>
                        </div>
                      ) : viewMode === "grid" ? (
                        <FileGrid
                          files={sortedFiles}
                          selectedFiles={selectedFiles}
                          onSelectFile={handleSelectFile}
                          onOpenFile={handleOpenFile}
                          onEditFile={handleEditFile}
                          onDownloadFile={handleDownloadFile}
                          onDeleteFile={(file) => void deleteOne(file)}
                          onCutSelected={() => setClipboardFromSelection("move")}
                          onCopySelected={() => setClipboardFromSelection("copy")}
                          canPaste={!!clipboard}
                          onPaste={(targetDir) => void pasteInto(targetDir)}
                        />
                      ) : (
                        <FileTable
                          files={sortedFiles}
                          selectedFiles={selectedFiles}
                          onSelectFile={handleSelectFile}
                          onOpenFile={handleOpenFile}
                          onEditFile={handleEditFile}
                          onDownloadFile={handleDownloadFile}
                          onDeleteFile={(file) => void deleteOne(file)}
                          sortField={sortField}
                          sortDirection={sortDirection}
                          onSortChange={toggleSort}
                          onCutSelected={() => setClipboardFromSelection("move")}
                          onCopySelected={() => setClipboardFromSelection("copy")}
                          canPaste={!!clipboard}
                          onPaste={(targetDir) => void pasteInto(targetDir)}
                        />
                      )}
                    </div>
                  </ContextMenuTrigger>
                  <ContextMenuContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                    <ContextMenuItem
                      disabled={!clipboard}
                      onClick={() => {
                        void pasteInto();
                      }}
                    >
                      Paste
                      <ContextMenuShortcut>⌘V</ContextMenuShortcut>
                    </ContextMenuItem>
                    <ContextMenuSeparator />
                    <ContextMenuItem
                      onClick={() => {
                        setSelectedFiles(new Set(filteredFiles.map((file) => file.id)));
                      }}
                    >
                      Select all
                      <ContextMenuShortcut>⌘A</ContextMenuShortcut>
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>

                {/* Footer path (Inside Left Panel) */}
                {/* Footer path (Editable) */}
                <div className="border-t bg-muted/30 h-9 px-3 flex items-center shrink-0">
                  {isEditingPath ? (
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        if (pathInput.trim() !== currentPath) {
                          handleNavigate(pathInput.trim());
                        }
                        setIsEditingPath(false);
                      }}
                      className="flex w-full"
                    >
                      <Input
                        autoFocus
                        value={pathInput}
                        onChange={(e) => setPathInput(e.target.value)}
                        onBlur={() => setIsEditingPath(false)}
                        className="h-7 text-xs bg-background w-full border-0 focus-visible:ring-0 focus-visible:ring-offset-0 shadow-none px-2"
                      />
                    </form>
                  ) : (
                    <ContextMenu>
                      <ContextMenuTrigger asChild>
                        <button
                          className="w-full text-left truncate text-xs text-muted-foreground hover:bg-black/5 dark:hover:bg-white/5 rounded px-2 py-1 transition-colors cursor-text"
                          onClick={() => {
                            setPathInput(currentPath);
                            setIsEditingPath(true);
                          }}
                          title="Click to edit path"
                        >
                          {currentPath}
                        </button>
                      </ContextMenuTrigger>
                      <ContextMenuContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                        <ContextMenuItem
                          className="hover:bg-sidebar-accent/30 hover:text-foreground focus:bg-sidebar-accent/30 focus:text-foreground"
                          onClick={() => {
                            navigator.clipboard?.writeText(currentPath).catch(() => { });
                          }}
                        >
                          Copy path
                        </ContextMenuItem>
                      </ContextMenuContent>
                    </ContextMenu>
                  )}
                </div>

                <UploadZone
                  ref={uploadZoneRef}
                  onUpload={handleUpload}
                  isDragging={isDragging}
                />
              </div>
            </Panel>

            {previewFile && (
              <>
                <PanelResizeHandle className="group relative flex w-1 items-center justify-center bg-transparent cursor-col-resize transition-colors focus:outline-none z-10 -ml-0.5">
                  <div className="h-full w-[1px] bg-border group-hover:bg-foreground/50 transition-colors" />
                </PanelResizeHandle>
                <Panel defaultSize={30} minSize={20} maxSize={60} className="bg-background/50">
                  <FilePreviewPanel
                    file={previewFile}
                    sourceId={sourceId}
                    onClose={() => {
                      setPreviewFile(null);
                      setEditTargetId(null);
                    }}
                    startInEditMode={editTargetId === previewFile.id}
                    onEditModeChange={(editing) => {
                      setEditTargetId(editing ? previewFile.id : null);
                    }}
                    onDownload={() => {
                      if (!previewFile) return;
                      void downloadOne(previewFile);
                    }}
                  />
                </Panel>
              </>
            )}
          </PanelGroup>
        </div>
      </div>

      <AlertDialog open={showDeleteConfirm} onOpenChange={setShowDeleteConfirm}>
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Delete selected items?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete the selected files and folders from{" "}
              <span className="font-medium">{storageName}</span>. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                void (async () => {
                  await handleBulkDelete();
                  setShowDeleteConfirm(false);
                })();
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={!!pasteConflict}
        onOpenChange={(open) => {
          if (!open) setPasteConflict(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Item already exists</AlertDialogTitle>
            <AlertDialogDescription>
              One or more items with the same name already exist in this location. Do you want to overwrite them or discard this copy?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-muted text-foreground hover:bg-muted/80"
              onClick={() => {
                if (!pasteConflict) return;
                void (async () => {
                  try {
                    await transferEntries(
                      pasteConflict.fromSourceId,
                      pasteConflict.toSourceId,
                      pasteConflict.paths,
                      pasteConflict.targetDir,
                      pasteConflict.operation,
                      "skip",
                    );
                    await loadFiles(currentPath);
                    toast({
                      title: "Paste completed",
                      description: "Existing items were discarded.",
                    });
                  } catch (error) {
                    toast({
                      title: "Paste failed",
                      description: error instanceof Error ? error.message : String(error),
                      variant: "destructive",
                    });
                  } finally {
                    setPasteConflict(null);
                  }
                })();
              }}
            >
              Discard
            </AlertDialogAction>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                if (!pasteConflict) return;
                void (async () => {
                  try {
                    await transferEntries(
                      pasteConflict.fromSourceId,
                      pasteConflict.toSourceId,
                      pasteConflict.paths,
                      pasteConflict.targetDir,
                      pasteConflict.operation,
                      "overwrite",
                    );
                    await loadFiles(currentPath);
                    toast({
                      title: "Paste completed",
                      description: "Existing items were overwritten.",
                    });
                  } catch (error) {
                    toast({
                      title: "Paste failed",
                      description: error instanceof Error ? error.message : String(error),
                      variant: "destructive",
                    });
                  } finally {
                    setPasteConflict(null);
                  }
                })();
              }}
            >
              Overwrite
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
