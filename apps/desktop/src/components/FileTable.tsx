import { FileItem } from "@/types/storage";
import { MoreVertical, Edit, Eye, Download, Trash2 } from "lucide-react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { getFileIcon, getFileColor } from "./FileIcon";

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
  onSelectFile: (fileId: string) => void;
  onSelectAll: () => void;
  onOpenFile?: (file: FileItem) => void;
  onDownloadFile?: (file: FileItem) => void;
  onDeleteFile?: (file: FileItem) => void;
  sortField?: "name" | "type" | "modified" | "size";
  sortDirection?: "asc" | "desc";
  onSortChange?: (field: "name" | "type" | "modified" | "size") => void;
}

export function FileTable({
  files,
  selectedFiles,
  onSelectFile,
  onSelectAll,
  onOpenFile,
  onDownloadFile,
  onDeleteFile,
  sortField = "name",
  sortDirection = "asc",
  onSortChange,
}: FileTableProps) {
  const allSelected = files.length > 0 && selectedFiles.size === files.length;
  const sortIndicator = (field: "name" | "type" | "modified" | "size") =>
    sortField === field ? (sortDirection === "asc" ? " ▲" : " ▼") : "";

  return (
    <div className="rounded-md border bg-card">
      <Table className="table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead className="w-12">
              <Checkbox checked={allSelected} onCheckedChange={onSelectAll} />
            </TableHead>
            <TableHead
              className="w-[32%] min-w-[5ch] cursor-pointer select-none whitespace-nowrap"
              onClick={() => onSortChange?.("name")}
            >
              Name{sortIndicator("name")}
            </TableHead>
            <TableHead
              className="w-[14%] min-w-[34ch] cursor-pointer select-none whitespace-nowrap"
              onClick={() => onSortChange?.("type")}
            >
              Type{sortIndicator("type")}
            </TableHead>
            <TableHead
              className="min-w-[34ch] cursor-pointer select-none whitespace-nowrap"
              onClick={() => onSortChange?.("modified")}
            >
              Modified{sortIndicator("modified")}
            </TableHead>
            <TableHead
              className="min-w-[34ch] cursor-pointer select-none text-right whitespace-nowrap"
              onClick={() => onSortChange?.("size")}
            >
              Size{sortIndicator("size")}
            </TableHead>
            <TableHead className="w-12" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {files.map((file) => {
            const Icon = getFileIcon(file);
            const color = getFileColor(file);
            const isSelected = selectedFiles.has(file.id);

            return (
              <TableRow
                key={file.id}
                className={`group cursor-pointer ${
                  isSelected ? "bg-muted/50" : "hover:bg-muted/50"
                }`}
                onDoubleClick={() => onOpenFile?.(file)}
              >
                <TableCell>
                  <Checkbox
                    checked={isSelected}
                    onCheckedChange={() => onSelectFile(file.id)}
                  />
                </TableCell>
                <TableCell className="w-[32%] min-w-[5ch] align-top">
                  <div className="flex items-start gap-3">
                    <Icon className={`h-5 w-5 shrink-0 ${color}`} />
                    <span className="block truncate text-sm font-medium" title={file.name}>
                      {file.name}
                    </span>
                  </div>
                </TableCell>
                <TableCell className="w-[14%] min-w-[34ch] truncate text-xs text-muted-foreground align-top">
                  {file.type === "folder" ? "Folder" : file.extension?.toUpperCase() || "-"}
                </TableCell>
                <TableCell className="min-w-[34ch] truncate text-muted-foreground">
                  {formatDate(file.modified || null)}
                </TableCell>
                <TableCell className="min-w-[34ch] truncate text-right text-muted-foreground">
                  {formatFileSize(file.size)}
                </TableCell>
                <TableCell>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 opacity-0 group-hover:opacity-100"
                      >
                        <MoreVertical className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent
                      align="end"
                      className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md"
                    >
                      {file.type === "file" && (
                        <>
                          <DropdownMenuItem onClick={() => onOpenFile?.(file)}>
                            <Eye className="mr-2 h-4 w-4" />
                            Preview
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => onDownloadFile?.(file)}>
                            <Download className="mr-2 h-4 w-4" />
                            Download
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => console.log("Edit", file.id)}>
                            <Edit className="mr-2 h-4 w-4" />
                            Edit
                          </DropdownMenuItem>
                        </>
                      )}
                      <DropdownMenuItem
                        onClick={() => onDeleteFile?.(file)}
                        className="text-destructive"
                      >
                        <Trash2 className="mr-2 h-4 w-4" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}
