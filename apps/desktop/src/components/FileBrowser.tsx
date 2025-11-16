import { useEffect, useState } from "react";
import { Search, Grid3x3, List, ChevronRight, Home, Download, Trash2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { FileGrid } from "./FileGrid";
import { FileTable } from "./FileTable";
import { FilePreviewDialog } from "./FilePreviewDialog";
import { FileItem } from "@/types/storage";
import { Entry, listEntries, readFile, deletePath, TauriApiError } from "@/lib/api";
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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  type SortField = "name" | "type" | "modified" | "size";
  type SortDirection = "asc" | "desc";

  const [sortField, setSortField] = useState<SortField>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const [previewFile, setPreviewFile] = useState<FileItem | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);

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
  }, [sourceId]);

  useEffect(() => {
    void loadFiles(currentPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPath, sourceId]);

  const handleNavigate = (path: string) => {
    setSearchQuery("");
    setSelectedFiles(new Set());
    setCurrentPath(path || "/");
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
      setPreviewOpen(true);
    }
  };

  const handleDownloadFile = (file: FileItem) => {
    void downloadOne(file);
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

  return (
    <div className="flex h-full flex-col bg-background">
      {/* Header */}
      <div className="border-b bg-card">
        <div className="flex items-center gap-4 px-6 py-4">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search files and folders..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-10"
            />
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="icon"
              variant={viewMode === 'grid' ? 'default' : 'ghost'}
              onClick={() => setViewMode('grid')}
            >
              <Grid3x3 className="h-4 w-4" />
            </Button>
            <Button
              size="icon"
              variant={viewMode === 'table' ? 'default' : 'ghost'}
              onClick={() => setViewMode('table')}
            >
              <List className="h-4 w-4" />
            </Button>
          </div>
        </div>

        {/* Breadcrumbs */}
        <div className="flex items-center gap-2 border-t px-6 py-3 text-sm">
          <Home className="h-4 w-4 text-muted-foreground" />
          {getBreadcrumbs().map((crumb, index) => (
            <div key={crumb.path} className="flex items-center gap-2">
              {index === 0 ? null : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
              <button
                className="hover:text-primary transition-colors"
                onClick={() => handleNavigate(crumb.path)}
              >
                {index === 0 ? storageName : crumb.name}
              </button>
            </div>
          ))}
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
            className={`border-b px-6 py-3 flex items-center justify-between h-11 ${
              hasSelection ? "bg-muted" : "bg-card"
            }`}
          >
            {hasSelection ? (
              <>
                <span className="text-sm font-medium">
                  {selectedFiles.size} item{selectedFiles.size > 1 ? "s" : ""} selected
                </span>
                <div className="flex gap-2">
                  <Button size="sm" variant="outline" onClick={handleBulkDownload}>
                    <Download className="mr-2 h-4 w-4" />
                    Download
                  </Button>
                  <Button size="sm" variant="destructive" onClick={handleBulkDelete}>
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
      <FilePreviewDialog
        open={previewOpen}
        onOpenChange={(open) => {
          setPreviewOpen(open);
          if (!open) setPreviewFile(null);
        }}
        file={previewFile}
        sourceId={sourceId}
      />
    </div>
  );
}
