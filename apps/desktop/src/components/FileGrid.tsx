import { FileItem } from "@/types/storage";
import { Eye, Download, Trash2, Edit3 } from "lucide-react";
import { FileTypeIcon } from "./FileIcon";
import { Card } from "@/components/ui/card";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";

const formatFileSize = (bytes?: number) => {
  if (!bytes) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

const GRID_MIN_COLUMN_WIDTH = 96;
const GRID_GAP = 8;
const GRID_PADDING = 4;
const GRID_ROW_ESTIMATE = 112;
const FILE_NAME_ELLIPSIS = "...";
const FILE_NAME_SUFFIX_LENGTH = 4;

const formatDisplayName = (name: string, maxChars: number) => {
  if (name.length <= maxChars) return name;
  if (maxChars <= FILE_NAME_ELLIPSIS.length + 1) {
    return name.slice(0, maxChars);
  }

  const suffixLength = Math.min(
    FILE_NAME_SUFFIX_LENGTH,
    Math.max(1, maxChars - FILE_NAME_ELLIPSIS.length),
  );
  const prefixLength = maxChars - FILE_NAME_ELLIPSIS.length - suffixLength;

  if (prefixLength <= 0) {
    return `${FILE_NAME_ELLIPSIS}${name.slice(-suffixLength)}`;
  }

  return `${name.slice(0, prefixLength)}${FILE_NAME_ELLIPSIS}${name.slice(-suffixLength)}`;
};

interface FileGridProps {
  files: FileItem[];
  selectedFiles: Set<string>;
  onSelectFile: (fileId: string, options?: { toggle?: boolean }) => void;
  onOpenFile?: (file: FileItem) => void;
  onEditFile?: (file: FileItem) => void;
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
  onEditFile,
  onDownloadFile,
  onDeleteFile,
}: FileGridProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState(3);
  const [nameMaxChars, setNameMaxChars] = useState(24);

  // Calculate columns based on container width
  useEffect(() => {
    const updateColumns = () => {
      if (parentRef.current) {
        const width = parentRef.current.offsetWidth;
        const calculatedColumns = Math.max(
          1,
          Math.floor((width + GRID_GAP) / (GRID_MIN_COLUMN_WIDTH + GRID_GAP)),
        );
        setColumns(calculatedColumns);

        const totalGaps = GRID_GAP * Math.max(0, calculatedColumns - 1);
        const availableWidth = Math.max(0, width - GRID_PADDING * 2 - totalGaps);
        const estimatedColumnWidth = availableWidth / calculatedColumns;
        const columnWidth = Math.min(estimatedColumnWidth, GRID_MIN_COLUMN_WIDTH);
        const approxCharsPerLine = Math.max(8, Math.floor(columnWidth / 7));
        setNameMaxChars(Math.max(12, approxCharsPerLine * 2));
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
    estimateSize: () => GRID_ROW_ESTIMATE,
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
              data-index={virtualRow.index}
              ref={rowVirtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                transform: `translateY(${virtualRow.start}px)`,
                display: "grid",
                gridTemplateColumns: `repeat(${columns}, minmax(${GRID_MIN_COLUMN_WIDTH}px, 1fr))`,
                gap: `${GRID_GAP}px`,
                padding: `${GRID_PADDING}px`,
              }}
            >
              {rowFiles.map((file) => {
                const isSelected = selectedFiles.has(file.id);
                const displayName = formatDisplayName(file.name, nameMaxChars);

                return (
                  <ContextMenu key={file.id}>
                    <ContextMenuTrigger asChild>
                      <Card
                        className={`group relative cursor-pointer overflow-hidden border-transparent font-normal text-foreground shadow-none transition-colors ${
                          isSelected ? "bg-sidebar-accent" : "bg-transparent hover:bg-sidebar-accent/50"
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
                        <div className="flex flex-col items-center gap-1 p-2 pb-1">
                          <div className="relative w-full flex justify-center">
                            <FileTypeIcon item={file} className="h-8 w-8" />
                          </div>

                          <div className="w-full text-center">
                            <p
                              className="line-clamp-2 break-words text-xs font-normal leading-tight text-foreground"
                              title={file.name}
                            >
                              {displayName}
                            </p>
                            {file.size && (
                              <p className="mt-0.5 text-[10px] font-normal text-muted-foreground">
                                {formatFileSize(file.size)}
                              </p>
                            )}
                          </div>
                        </div>
                      </Card>
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
            </div>
          );
        })}
      </div>
    </div>
  );
}
