import { FileItem } from "@/types/storage";
import { MoreVertical, Edit, Eye, Download, Trash2 } from "lucide-react";
import { getFileIcon, getFileColor } from "./FileIcon";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const formatFileSize = (bytes?: number) => {
  if (!bytes) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

interface FileGridProps {
  files: FileItem[];
  selectedFiles: Set<string>;
  onSelectFile: (fileId: string) => void;
  onOpenFile?: (file: FileItem) => void;
  onDownloadFile?: (file: FileItem) => void;
  onDeleteFile?: (file: FileItem) => void;
}

import { useRef, useEffect, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

export function FileGrid({
  files,
  selectedFiles,
  onSelectFile,
  onOpenFile,
  onDownloadFile,
  onDeleteFile,
}: FileGridProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState(3);

  // Calculate columns based on container width
  useEffect(() => {
    const updateColumns = () => {
      if (parentRef.current) {
        const width = parentRef.current.offsetWidth;
        const minColumnWidth = 140; // Minimum width for a card
        const gap = 16; // Gap between items
        // Calculate how many columns fit
        const calculatedColumns = Math.max(1, Math.floor((width + gap) / (minColumnWidth + gap)));
        setColumns(calculatedColumns);
      }
    };

    updateColumns();

    const observer = new ResizeObserver(updateColumns);
    if (parentRef.current) {
      observer.observe(parentRef.current);
    }

    return () => observer.disconnect();
  }, []);

  const rowCount = Math.ceil(files.length / columns);

  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 115, // Further reduced height (~10%)
    overscan: 5,
  });

  return (
    <div
      ref={parentRef}
      className="h-full w-full overflow-auto"
    >
      <div
        style={{
          height: `${rowVirtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {rowVirtualizer.getVirtualItems().map((virtualRow) => {
          const startIndex = virtualRow.index * columns;
          const rowFiles = files.slice(startIndex, startIndex + columns);

          return (
            <div
              key={virtualRow.index}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: `${virtualRow.size}px`,
                transform: `translateY(${virtualRow.start}px)`,
                display: "grid",
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                gap: "1rem",
                padding: "0.75rem", // p-3
              }}
            >
              {rowFiles.map((file) => {
                const Icon = getFileIcon(file);
                const color = getFileColor(file);
                const isSelected = selectedFiles.has(file.id);

                return (
                  <Card
                    key={file.id}
                    className={`group relative cursor-pointer overflow-hidden transition-all hover:shadow-md ${isSelected ? "border-primary ring-2 ring-primary/20" : "hover:border-primary/50"
                      }`}
                    onDoubleClick={() => onOpenFile?.(file)}
                  >
                    <Checkbox
                      checked={isSelected}
                      onCheckedChange={() => onSelectFile(file.id)}
                      className="absolute left-1 top-1 z-10 opacity-0 group-hover:opacity-100 data-[state=checked]:opacity-100"
                      onClick={(e) => e.stopPropagation()}
                    />

                    <div className="flex flex-col items-center gap-1 p-2 pb-1">
                      <div className="relative w-full flex justify-center">
                        <Icon className={`h-8 w-8 ${color}`} />
                      </div>

                      <div className="w-full text-center">
                        <p className="mx-auto max-w-[12ch] truncate text-xs font-medium" title={file.name}>
                          {file.name}
                        </p>
                        {file.size && (
                          <p className="mt-0.5 text-[10px] text-muted-foreground">
                            {formatFileSize(file.size)}
                          </p>
                        )}
                      </div>
                    </div>

                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          size="icon"
                          variant="ghost"
                          className="absolute right-1 top-1 h-6 w-6 opacity-0 transition-opacity group-hover:opacity-100"
                          onClick={(e) => e.stopPropagation()}
                        >
                          <MoreVertical className="h-3 w-3" />
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
                  </Card>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}

