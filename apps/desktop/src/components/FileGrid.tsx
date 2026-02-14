import { FileItem } from "@/types/storage";
import { Eye, Download, Trash2, Edit3, Scissors, Copy, ClipboardPaste } from "lucide-react";
import { FileTypeIcon } from "./FileIcon";
import { Card } from "@/components/ui/card";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
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
  sourceId: string;
  files: FileItem[];
  selectedFiles: Set<string>;
  onSelectFile: (fileId: string, options?: { toggle?: boolean }) => void;
  onSelectFiles?: (fileIds: string[]) => void;
  onOpenFile?: (file: FileItem) => void;
  onEditFile?: (file: FileItem) => void;
  onDownloadFile?: (file: FileItem) => void;
  onDeleteFile?: (file: FileItem) => void;
  onCutSelected?: () => void;
  onCopySelected?: () => void;
  canPaste?: boolean;
  onPaste?: (targetDir?: string) => void;
  onMoveToFolder?: (paths: string[], folderPath: string) => void;
  onClearSelection?: () => void;
}

import { useRef, useEffect, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

type InternalTransferPayload = {
  kind: "infimount-transfer";
  fromSourceId: string;
  paths: string[];
  operation?: "copy" | "move";
};

const INTERNAL_TRANSFER_MIME = "application/x-infimount-transfer";

const isExternalFileDrag = (dt: DataTransfer) => {
  const types = Array.from(dt.types ?? []);
  if (types.includes("Files")) return true;
  if (dt.files && dt.files.length > 0) return true;

  const items = Array.from(dt.items ?? []);
  return items.some((item) => item.kind === "file");
};

const isLikelyInternalTransferDrag = (dt: DataTransfer) => {
  const types = Array.from(dt.types ?? []);
  if (types.includes(INTERNAL_TRANSFER_MIME)) return true;
  if (isExternalFileDrag(dt)) return false;
  return types.includes("text/plain") || types.includes("Text");
};

const parseInternalTransfer = (dt: DataTransfer): InternalTransferPayload | null => {
  const raw =
    dt.getData(INTERNAL_TRANSFER_MIME)
    || dt.getData("text/plain");
  if (!raw) return null;

  try {
    const parsed = JSON.parse(raw) as Partial<InternalTransferPayload>;
    if (parsed.kind !== "infimount-transfer") return null;
    if (typeof parsed.fromSourceId !== "string" || parsed.fromSourceId.length === 0) return null;
    if (!Array.isArray(parsed.paths) || parsed.paths.length === 0) return null;
    const paths = parsed.paths.filter((p) => typeof p === "string") as string[];
    if (paths.length === 0) return null;
    return {
      kind: "infimount-transfer",
      fromSourceId: parsed.fromSourceId,
      paths,
      operation: parsed.operation === "move" ? "move" : "copy",
    };
  } catch {
    return null;
  }
};

export function FileGrid({
  sourceId,
  files,
  selectedFiles,
  onSelectFile,
  onSelectFiles,
  onOpenFile,
  onEditFile,
  onDownloadFile,
  onDeleteFile,
  onCutSelected,
  onCopySelected,
  canPaste,
  onPaste,
  onMoveToFolder,
  onClearSelection,
}: FileGridProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const [columns, setColumns] = useState(3);
  const [columnWidth, setColumnWidth] = useState(GRID_MIN_COLUMN_WIDTH);
  const [nameMaxChars, setNameMaxChars] = useState(24);
  const [folderDropTargetId, setFolderDropTargetId] = useState<string | null>(null);
  const [dragSelect, setDragSelect] = useState<{
    startX: number;
    startY: number;
    x: number;
    y: number;
  } | null>(null);

  // Calculate columns based on container width
  useEffect(() => {
    const updateColumns = () => {
      if (parentRef.current) {
        const width = Math.floor(parentRef.current.clientWidth);
        const usableWidth = Math.max(0, width - GRID_PADDING * 2);
        const calculatedColumns = Math.max(
          1,
          Math.floor((usableWidth + GRID_GAP) / (GRID_MIN_COLUMN_WIDTH + GRID_GAP)),
        );
        const totalGaps = GRID_GAP * Math.max(0, calculatedColumns - 1);
        const availableWidth = Math.max(0, usableWidth - totalGaps);
        const computedColumnWidth = Math.max(1, Math.floor(availableWidth / calculatedColumns));
        const approxCharsPerLine = Math.max(8, Math.floor(computedColumnWidth / 7));

        setColumns(calculatedColumns);
        setColumnWidth(computedColumnWidth);
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

  useEffect(() => {
    if (!dragSelect || !onSelectFiles || !parentRef.current) return;

    const container = parentRef.current;
    const containerRect = container.getBoundingClientRect();
    const scrollLeft = container.scrollLeft;
    const scrollTop = container.scrollTop;

    const left = Math.min(dragSelect.startX, dragSelect.x);
    const right = Math.max(dragSelect.startX, dragSelect.x);
    const top = Math.min(dragSelect.startY, dragSelect.y);
    const bottom = Math.max(dragSelect.startY, dragSelect.y);

    const selected: string[] = [];
    itemRefs.current.forEach((element, id) => {
      const rect = element.getBoundingClientRect();
      const itemLeft = rect.left - containerRect.left + scrollLeft;
      const itemRight = itemLeft + rect.width;
      const itemTop = rect.top - containerRect.top + scrollTop;
      const itemBottom = itemTop + rect.height;

      const intersects =
        itemLeft < right &&
        itemRight > left &&
        itemTop < bottom &&
        itemBottom > top;

      if (intersects) {
        selected.push(id);
      }
    });

    onSelectFiles(selected);
  }, [dragSelect, onSelectFiles]);

  return (
    <div
      ref={parentRef}
      className="h-full w-full overflow-y-auto overflow-x-hidden"
      onMouseDown={(event) => {
        if (event.button !== 0) return;
        const target = event.target as HTMLElement | null;
        if (!target) return;
        if (target.closest('[data-infimount-file-item="true"]')) return;
        if (!parentRef.current) return;
        const rect = parentRef.current.getBoundingClientRect();
        const x = event.clientX - rect.left + parentRef.current.scrollLeft;
        const y = event.clientY - rect.top + parentRef.current.scrollTop;
        setDragSelect({ startX: x, startY: y, x, y });
        onClearSelection?.();
        event.preventDefault();
      }}
      onMouseMove={(event) => {
        if (!dragSelect || !parentRef.current) return;
        const rect = parentRef.current.getBoundingClientRect();
        const x = event.clientX - rect.left + parentRef.current.scrollLeft;
        const y = event.clientY - rect.top + parentRef.current.scrollTop;
        setDragSelect((prev) => (prev ? { ...prev, x, y } : prev));
        event.preventDefault();
      }}
      onMouseUp={() => {
        if (dragSelect) {
          setDragSelect(null);
        }
      }}
      onMouseLeave={(event) => {
        if (!dragSelect || (event.buttons & 1) !== 1) return;
        setDragSelect(null);
      }}
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

          const rowTop = Math.round(virtualRow.start);

          return (
            <div
              key={virtualRow.index}
              data-index={virtualRow.index}
              ref={rowVirtualizer.measureElement}
              style={{
                position: "absolute",
                top: `${rowTop}px`,
                left: 0,
                width: "100%",
                boxSizing: "border-box",
                display: "grid",
                gridTemplateColumns: `repeat(${columns}, ${columnWidth}px)`,
                gap: `${GRID_GAP}px`,
                padding: `${GRID_PADDING}px`,
                justifyContent: "start",
              }}
            >
              {rowFiles.map((file) => {
                const isSelected = selectedFiles.has(file.id);
                const displayName = formatDisplayName(file.name, nameMaxChars);

                return (
                  <ContextMenu key={file.id}>
                    <ContextMenuTrigger asChild>
                      <Card
                        ref={(node) => {
                          if (node) {
                            itemRefs.current.set(file.id, node);
                          } else {
                            itemRefs.current.delete(file.id);
                          }
                        }}
                        data-infimount-file-item="true"
                        className={`group relative cursor-pointer border-transparent font-normal text-foreground shadow-none antialiased transition-all duration-200 ${isSelected
                          ? "bg-primary/15 ring-1 ring-primary/20"
                          : "bg-transparent hover:bg-black/5 dark:hover:bg-white/5"
                          } ${file.type === "folder" && folderDropTargetId === file.id && !isSelected
                            ? "bg-primary/10 ring-1 ring-primary/30"
                            : ""
                          }`}
                        draggable
                        onDragStart={(event) => {
                          const paths = selectedFiles.has(file.id)
                            ? Array.from(selectedFiles)
                            : [file.id];

                          if (!selectedFiles.has(file.id)) {
                            onSelectFile(file.id);
                          }

                          const payload = JSON.stringify({
                            kind: "infimount-transfer",
                            fromSourceId: sourceId,
                            paths,
                            operation: "copy",
                          });
                          event.dataTransfer.setData(INTERNAL_TRANSFER_MIME, payload);
                          event.dataTransfer.setData("text/plain", payload);
                          event.dataTransfer.effectAllowed = "copyMove";
                        }}
                        onDragOver={(event) => {
                          if (file.type !== "folder" || !onMoveToFolder) return;
                          if (!isLikelyInternalTransferDrag(event.dataTransfer)) return;
                          event.preventDefault();
                          event.stopPropagation();
                          event.dataTransfer.dropEffect = "move";
                          setFolderDropTargetId(file.id);
                        }}
                        onDragLeave={() => {
                          if (file.type !== "folder" || !onMoveToFolder) return;
                          setFolderDropTargetId((prev) => (prev === file.id ? null : prev));
                        }}
                        onDrop={(event) => {
                          if (file.type !== "folder" || !onMoveToFolder) return;
                          event.preventDefault();
                          event.stopPropagation();
                          setFolderDropTargetId(null);

                          const payload = parseInternalTransfer(event.dataTransfer);
                          if (!payload) return;
                          if (payload.fromSourceId !== sourceId) return;
                          onMoveToFolder(payload.paths, file.id);
                        }}
                        onDoubleClick={() => onOpenFile?.(file)}
                        onClick={(event) => {
                          const toggle = event.metaKey || event.ctrlKey;
                          if (toggle) {
                            onSelectFile(file.id, { toggle: true });
                          } else {
                            onSelectFile(file.id);
                          }
                        }}
                        onContextMenu={(event) => {
                          // Prevent the background (destination) context menu from opening.
                          event.stopPropagation();
                          if (!selectedFiles.has(file.id)) {
                            onSelectFile(file.id);
                          }
                        }}
                      >
                        <div className="flex flex-col items-center gap-1 p-2 pb-1">
                          <div className="flex h-9 w-9 items-center justify-center">
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
                      {(onCutSelected || onCopySelected || onPaste) && (
                        <>
                          {onCutSelected && (
                            <ContextMenuItem onClick={onCutSelected}>
                              <Scissors className="mr-2 h-4 w-4" />
                              Cut
                              <ContextMenuShortcut>⌘X</ContextMenuShortcut>
                            </ContextMenuItem>
                          )}
                          {onCopySelected && (
                            <ContextMenuItem onClick={onCopySelected}>
                              <Copy className="mr-2 h-4 w-4" />
                              Copy
                              <ContextMenuShortcut>⌘C</ContextMenuShortcut>
                            </ContextMenuItem>
                          )}
                          {onPaste && (
                            <ContextMenuItem
                              disabled={!canPaste}
                              onClick={() => {
                                const targetDir = file.type === "folder" ? file.id : undefined;
                                onPaste(targetDir);
                              }}
                            >
                              <ClipboardPaste className="mr-2 h-4 w-4" />
                              {file.type === "folder" ? "Paste into folder" : "Paste"}
                              <ContextMenuShortcut>⌘V</ContextMenuShortcut>
                            </ContextMenuItem>
                          )}
                          <ContextMenuSeparator />
                        </>
                      )}
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
          );
        })}
        {dragSelect && (
          <div
            className="pointer-events-none absolute z-20 rounded-sm border border-primary/60 bg-primary/15"
            style={{
              left: `${Math.min(dragSelect.startX, dragSelect.x)}px`,
              top: `${Math.min(dragSelect.startY, dragSelect.y)}px`,
              width: `${Math.abs(dragSelect.x - dragSelect.startX)}px`,
              height: `${Math.abs(dragSelect.y - dragSelect.startY)}px`,
            }}
          />
        )}
      </div>
    </div>
  );
}
