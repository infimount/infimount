import { FileItem } from "@/types/storage";
import { Eye, Download, Trash2, Edit3 } from "lucide-react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { FileTypeIcon } from "./FileIcon";

const formatFileSize = (bytes?: number) => {
  if (!bytes) return "-";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

const formatDate = (date: Date | null) => {
  if (!date) return "-";
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));

  if (days === 0) {
    return date.toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit" });
  }
  if (days === 1) {
    return "Yesterday";
  }
  if (days < 7) {
    return `${days} days ago`;
  }
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
};

interface FileTableProps {
  files: FileItem[];
  selectedFiles: Set<string>;
  onSelectFile: (fileId: string, options?: { toggle?: boolean }) => void;
  onOpenFile?: (file: FileItem) => void;
  onEditFile?: (file: FileItem) => void;
  onDownloadFile?: (file: FileItem) => void;
  onDeleteFile?: (file: FileItem) => void;
  sortField?: "name" | "type" | "modified" | "size";
  sortDirection?: "asc" | "desc";
  onSortChange?: (field: "name" | "type" | "modified" | "size") => void;
}

import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

export function FileTable({
  files,
  selectedFiles,
  onSelectFile,
  onOpenFile,
  onEditFile,
  onDownloadFile,
  onDeleteFile,
  sortField = "name",
  sortDirection = "asc",
  onSortChange,
}: FileTableProps) {
  const sortIndicator = (field: "name" | "type" | "modified" | "size") =>
    sortField === field ? (sortDirection === "asc" ? " ▲" : " ▼") : "";

  const parentRef = useRef<HTMLDivElement>(null);

  const rowVirtualizer = useVirtualizer({
    count: files.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 53, // Approximate height of a table row
    overscan: 10,
  });

  const { getVirtualItems, getTotalSize } = rowVirtualizer;
  const virtualItems = getVirtualItems();
  const paddingTop = virtualItems.length > 0 ? virtualItems[0].start : 0;
  const paddingBottom =
    virtualItems.length > 0
      ? getTotalSize() - virtualItems[virtualItems.length - 1].end
      : 0;

  return (
    <div
      ref={parentRef}
      className="h-full w-full overflow-auto rounded-md border bg-card"
    >
      <Table className="table-fixed">
        <TableHeader className="sticky top-0 z-10 bg-card shadow-sm">
          <TableRow>
            <TableHead
              className="h-10 w-[48%] min-w-[12ch] cursor-pointer select-none whitespace-nowrap px-3"
              onClick={() => onSortChange?.("name")}
            >
              Name{sortIndicator("name")}
            </TableHead>
            <TableHead
              className="h-10 w-[14%] min-w-[10ch] cursor-pointer select-none whitespace-nowrap px-3"
              onClick={() => onSortChange?.("type")}
            >
              Type{sortIndicator("type")}
            </TableHead>
            <TableHead
              className="h-10 w-[22%] min-w-[16ch] cursor-pointer select-none whitespace-nowrap px-3"
              onClick={() => onSortChange?.("modified")}
            >
              Modified{sortIndicator("modified")}
            </TableHead>
            <TableHead
              className="h-10 w-[16%] min-w-[8ch] cursor-pointer select-none text-right whitespace-nowrap px-3"
              onClick={() => onSortChange?.("size")}
            >
              Size{sortIndicator("size")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {paddingTop > 0 && (
            <TableRow>
              <TableCell colSpan={4} style={{ height: `${paddingTop}px`, padding: 0 }} />
            </TableRow>
          )}
          {virtualItems.map((virtualRow) => {
            const file = files[virtualRow.index];
            const isSelected = selectedFiles.has(file.id);

            return (
              <ContextMenu key={file.id}>
                <ContextMenuTrigger asChild>
                  <TableRow
                    className={`group cursor-pointer ${isSelected ? "bg-muted/50" : "hover:bg-muted/50"
                      }`}
                    onDoubleClick={() => onOpenFile?.(file)}
                    onClick={(event) => {
                      const toggle = event.metaKey || event.ctrlKey;
                      if (toggle) {
                        onSelectFile(file.id, { toggle: true });
                      } else {
                        onSelectFile(file.id);
                      }
                    }}
                    onContextMenu={() => {
                      if (!selectedFiles.has(file.id)) {
                        onSelectFile(file.id);
                      }
                    }}
                  >
                    <TableCell className="w-[48%] min-w-[12ch] align-top px-3 py-2">
                      <div className="flex items-start gap-3">
                        <FileTypeIcon item={file} className="h-5 w-5 shrink-0" />
                        <span className="block truncate text-sm font-medium" title={file.name}>
                          {file.name}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="w-[14%] min-w-[10ch] truncate text-xs text-muted-foreground align-top px-3 py-2">
                      {file.type === "folder" ? "Folder" : file.extension?.toUpperCase() || "-"}
                    </TableCell>
                    <TableCell className="w-[22%] min-w-[16ch] truncate text-muted-foreground px-3 py-2">
                      {formatDate(file.modified || null)}
                    </TableCell>
                    <TableCell className="w-[16%] min-w-[8ch] truncate text-right text-muted-foreground px-3 py-2">
                      {formatFileSize(file.size)}
                    </TableCell>
                  </TableRow>
                </ContextMenuTrigger>
                <ContextMenuContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                  {file.type === "file" && (
                    <>
                      <ContextMenuItem onClick={() => onOpenFile?.(file)}>
                        <Eye className="mr-2 h-4 w-4" />
                        Preview
                      </ContextMenuItem>
                      <ContextMenuItem onClick={() => onDownloadFile?.(file)}>
                        <Download className="mr-2 h-4 w-4" />
                        Download
                      </ContextMenuItem>
                      {onEditFile && (
                        <ContextMenuItem onClick={() => onEditFile(file)}>
                          <Edit3 className="mr-2 h-4 w-4" />
                          Edit
                        </ContextMenuItem>
                      )}
                    </>
                  )}
                  <ContextMenuItem
                    onClick={() => onDeleteFile?.(file)}
                    className="text-destructive"
                  >
                    <Trash2 className="mr-2 h-4 w-4" />
                    Delete
                  </ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            );
          })}
          {paddingBottom > 0 && (
            <TableRow>
              <TableCell colSpan={4} style={{ height: `${paddingBottom}px`, padding: 0 }} />
            </TableRow>
          )}
        </TableBody>
      </Table>
    </div>
  );
}
