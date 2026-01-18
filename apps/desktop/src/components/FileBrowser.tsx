import { useEffect, useState, useRef } from "react";
import {
  Search,
  LayoutGrid,
  LayoutList,
  ChevronLeft,
  ChevronRight,
  Upload,
  Download,
  Trash2,
  PanelLeft,
  PanelRight,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
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
}

interface LoadError {
  title: string;
  detail?: string;
}

export function FileBrowser({ sourceId, storageName, onPreviewVisibilityChange, onToggleSidebar }: FileBrowserProps) {
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
  const [editTargetId, setEditTargetId] = useState<string | null>(null);

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

  const handleSelectFile = (fileId: string) => {
    setSelectedFiles(prev => {
      const newSet = new Set(prev);
      if (newSet.has(fileId)) {
        newSet.delete(fileId);
      } else {
        newSet.add(fileId);
      }
      return newSet;
    });
  };

  const handleSelectAll = () => {
    if (selectedFiles.size === filteredFiles.length) {
      setSelectedFiles(new Set());
    } else {
      setSelectedFiles(new Set(filteredFiles.map(f => f.id)));
    }
  };

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

  const handleBulkDownload = async () => {
    const filesToDownload = filteredFiles.filter(
      (f) => selectedFiles.has(f.id) && f.type === "file",
    );
    for (const file of filesToDownload) {
      // eslint-disable-next-line no-await-in-loop
      await downloadOne(file);
    }
  };

  const deleteOne = async (file: FileItem) => {
    try {
      await deletePath(sourceId, file.id);
    } catch (error: any) {
      toast({
        title: "Delete failed",
        description: error?.message || String(error),
        variant: "destructive",
      });
    }
  };

  const handleBulkDelete = async () => {
    const toDelete = filteredFiles.filter((f) => selectedFiles.has(f.id));
    for (const file of toDelete) {
      // eslint-disable-next-line no-await-in-loop
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
        } catch (error: any) {
          toast({
            title: "Upload failed",
            description: error?.message || String(error),
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
  const fullPath = breadcrumbs.map((crumb) => crumb.name).join(" / ");
  const currentLabel =
    breadcrumbs[breadcrumbs.length - 1]?.name ?? storageName;

  const [isDragging, setIsDragging] = useState(false);
  const uploadZoneRef = useRef<UploadZoneRef | null>(null);

  return (
    <>
      <div
        className="relative flex h-full bg-background"
        onDragOver={(event: React.DragEvent<HTMLDivElement>) => {
          event.preventDefault();
          event.stopPropagation();
          setIsDragging(true);
        }}
        onDragLeave={(event: React.DragEvent<HTMLDivElement>) => {
          event.preventDefault();
          event.stopPropagation();
          setIsDragging(false);
        }}
        onDrop={(event: React.DragEvent<HTMLDivElement>) => {
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
            } catch (err) {
              // If folder support is not available, fall back gracefully.
              toast({
                title: "Upload failed",
                description:
                  (err as any)?.message ||
                  "Could not read some dropped items. Try dropping files only or use the file picker.",
                variant: "destructive",
              });
            }
          })();
        }}
      >
        <div className="flex flex-1 flex-col">
        {/* Header with navigation */}
        <div className="border-b bg-muted/30">
          <div className="flex items-center gap-2 px-4 py-3">
            <div className="flex items-center gap-1">
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8 mr-1"
                onClick={onToggleSidebar}
                title="Toggle Storage Sidebar"
              >
                <PanelLeft className="h-4 w-4" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                onClick={goBack}
                disabled={!canGoBack}
              >
                <ChevronLeft className="h-4 w-4" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                onClick={goForward}
                disabled={!canGoForward}
              >
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>

            <div className="flex min-w-0 flex-1 items-center gap-2">
              <span className="truncate text-sm font-medium">
                {currentLabel}
              </span>
            </div>

            <div className="flex items-center gap-2">
              <div className="relative w-48">
                <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder="Search..."
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  className="h-8 bg-background pl-9"
                />
              </div>

              <label htmlFor="file-upload">
                <Button
                  size="icon"
                  variant="outline"
                  className="h-8 w-8"
                  asChild
                >
                  <span>
                    <Upload className="h-4 w-4" />
                  </span>
                </Button>
              </label>
              <Button
                size="icon"
                variant="ghost"
                onClick={() =>
                  setViewMode((current) => (current === "grid" ? "table" : "grid"))
                }
                className="h-8 w-8"
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
                  variant="secondary"
                  onClick={() => setPreviewFile(null)}
                  className="h-8 w-8"
                  title="Close Preview"
                >
                  <PanelRight className="h-4 w-4" />
                </Button>
              )}
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

        {/* Bulk Actions Bar */}
        {selectedFiles.size > 0 && (
          <div className="flex h-11 items-center justify-between border-b border-border bg-accent/20 px-6 py-3">
            <span className="text-sm font-medium">
              {selectedFiles.size} item{selectedFiles.size > 1 ? "s" : ""} selected
            </span>
            <div className="flex gap-2">
              <Button size="sm" variant="outline" onClick={handleSelectAll}>
                Select all
              </Button>
              <Button size="sm" variant="outline" onClick={handleBulkDownload}>
                <Download className="mr-2 h-4 w-4" />
                Download
              </Button>
              <Button size="sm" variant="outline" onClick={() => setShowDeleteConfirm(true)}>
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </Button>
            </div>
          </div>
        )}

        {/* Content */}
        <div className="flex-1 overflow-hidden p-6">
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
            />
          ) : (
            <FileTable
              files={sortedFiles}
              selectedFiles={selectedFiles}
              onSelectFile={handleSelectFile}
              onSelectAll={handleSelectAll}
              onOpenFile={handleOpenFile}
              onEditFile={handleEditFile}
              onDownloadFile={handleDownloadFile}
              onDeleteFile={(file) => void deleteOne(file)}
              sortField={sortField}
              sortDirection={sortDirection}
              onSortChange={toggleSort}
            />
          )}
        </div>

        {/* Footer path */}
        <div className="border-t bg-muted/30 px-6 py-2">
          <p className="truncate text-xs text-muted-foreground">{fullPath}</p>
        </div>

        <UploadZone
          ref={uploadZoneRef}
          onUpload={handleUpload}
          isDragging={isDragging}
        />
      </div>

      {previewFile && (
        <div
          className="absolute inset-y-0 right-0 z-50 w-full border-l-2 border-border/60 bg-card md:relative md:block md:w-[30%] md:min-w-[250px] md:max-w-[600px]"
        >
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
        </div>
      )}
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
    </>
  );
}
