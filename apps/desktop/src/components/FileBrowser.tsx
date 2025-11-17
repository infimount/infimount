import { useEffect, useState } from "react";
import {
  Search,
  Grid3x3,
  List,
  ChevronLeft,
  ChevronRight,
  Upload,
  Download,
  Trash2,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { FileGrid } from "./FileGrid";
import { FileTable } from "./FileTable";
import { UploadZone } from "./UploadZone";
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
import { toast } from "@/hooks/use-toast";

interface FileBrowserProps {
  sourceId: string;
  storageName: string;
}

export function FileBrowser({ sourceId, storageName }: FileBrowserProps) {
  const [viewMode, setViewMode] = useState<"grid" | "table">("grid");
  const [searchQuery, setSearchQuery] = useState('');
  const [currentPath, setCurrentPath] = useState<string>("/");
  const [allFiles, setAllFiles] = useState<FileItem[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [history, setHistory] = useState<string[]>(["/"]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  type SortField = "name" | "type" | "modified" | "size";
  type SortDirection = "asc" | "desc";

  const [sortField, setSortField] = useState<SortField>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const [previewFile, setPreviewFile] = useState<FileItem | null>(null);

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
        setError(err.message);
      } else {
        setError("Failed to load files");
      }
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    setCurrentPath("/");
    setSelectedFiles(new Set());
    setHistory(["/"]);
    setHistoryIndex(0);
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
    if (!confirm("Delete selected items? This cannot be undone.")) return;
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
      setPreviewFile(file);
    }
  };

  const handleDownloadFile = (file: FileItem) => {
    void downloadOne(file);
  };

  const handleUpload = (files: File[]) => {
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

  const handlePreviewEdit = () => {
    if (!previewFile) return;
    toast({
      title: "Open in editor",
      description: "Opening files in an external editor is not implemented yet.",
    });
  };

  const handlePreviewDownload = () => {
    if (!previewFile) return;
    void downloadOne(previewFile);
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

  return (
    <div className="flex h-full bg-background relative">
      <div className="flex flex-1 flex-col">
        {/* Header with navigation */}
        <div className="border-b bg-card">
          <div className="flex items-center gap-2 px-4 py-3">
            <div className="flex items-center gap-1">
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
              <div className="relative w-64">
                <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder="Search files and folders..."
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  className="h-9 pl-9"
                />
              </div>

              <label htmlFor="file-upload">
                <Button
                  size="icon"
                  variant="outline"
                  className="h-9 w-9"
                  asChild
                >
                  <span>
                    <Upload className="h-4 w-4" />
                  </span>
                </Button>
              </label>

              <div className="flex items-center gap-1 rounded-md border">
                <Button
                  size="icon"
                  variant={viewMode === "grid" ? "default" : "ghost"}
                  onClick={() => setViewMode("grid")}
                  className="h-9 w-9"
                >
                  <Grid3x3 className="h-4 w-4" />
                </Button>
                <Button
                  size="icon"
                  variant={viewMode === "table" ? "default" : "ghost"}
                  onClick={() => setViewMode("table")}
                  className="h-9 w-9"
                >
                  <List className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </div>
        </div>

        {/* Error */}
        {error && (
          <div className="border-b bg-destructive/10 px-6 py-2 text-sm text-destructive">
            {error}
          </div>
        )}

        {/* Bulk Actions Bar – fixed height so the scroller doesn’t jump */}
        {(() => {
          const hasSelection = selectedFiles.size > 0;
          return (
            <div
              className={`flex h-11 items-center justify-between border-b px-6 py-3 ${
                hasSelection ? "bg-muted" : "bg-card"
              }`}
            >
              {hasSelection ? (
                <>
                  <span className="text-sm font-medium">
                    {selectedFiles.size} item
                    {selectedFiles.size > 1 ? "s" : ""} selected
                  </span>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={handleBulkDownload}
                    >
                      <Download className="mr-2 h-4 w-4" />
                      Download
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      onClick={handleBulkDelete}
                    >
                      <Trash2 className="mr-2 h-4 w-4" />
                      Delete
                    </Button>
                  </div>
                </>
              ) : null}
            </div>
          );
        })()}

        {/* Content */}
        <div className="flex-1 overflow-auto p-6">
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
              onDownloadFile={handleDownloadFile}
              onDeleteFile={(file) => void deleteOne(file)}
              sortField={sortField}
              sortDirection={sortDirection}
              onSortChange={toggleSort}
            />
          )}
        </div>

        {/* Footer path */}
        <div className="border-t bg-card px-6 py-2">
          <p className="truncate text-xs text-muted-foreground">{fullPath}</p>
        </div>

        <UploadZone onUpload={handleUpload} />
      </div>

      {previewFile && (
        <div className="w-80">
          <FilePreviewPanel
            file={previewFile}
            onClose={() => setPreviewFile(null)}
            sourceId={sourceId}
            onEdit={handlePreviewEdit}
            onDownload={handlePreviewDownload}
          />
        </div>
      )}
    </div>
  );
}
