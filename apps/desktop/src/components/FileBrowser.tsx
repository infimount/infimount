import { useCallback, useEffect, useMemo, useState, useRef, type CSSProperties } from "react";
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
  Copy,
  MoveRight,
  Star,
  Clock,
  Trash2,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { WindowControls } from "./WindowControls";
import { FileGrid } from "./FileGrid";
import { FileTable } from "./FileTable";
import { UploadZone, type UploadFileLike, type UploadZoneRef } from "./UploadZone";
import { FilePreviewPanel } from "./FilePreviewPanel";
import { TransferQueuePanel } from "./TransferQueuePanel";
import { FileItem } from "@/types/storage";
import {
  Entry,
  listEntriesPage,
  listEntriesRecursive,
  downloadFileToDownloads,
  writeFile,
  uploadFileStreaming,
  createDirectory,
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
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
import { useAppZoom } from "@/hooks/use-app-zoom";
import { useTransferQueue, type TransferJobRequest } from "@/hooks/use-transfer-queue";
import infinityLoader from "@/assets/loading-infinity.apng";

// Helper to extract file-like objects (including from dropped folders, where supported)
interface WebkitFileSystemEntry {
  name: string;
  isFile: boolean;
  isDirectory: boolean;
  file?: (
    successCallback: (file: File) => void,
    errorCallback?: (error: unknown) => void,
  ) => void;
  createReader?: () => {
    readEntries: (
      successCallback: (entries: WebkitFileSystemEntry[]) => void,
      errorCallback?: (error: unknown) => void,
    ) => void;
  };
}

type DataTransferItemWithWebkitEntry = Omit<DataTransferItem, "webkitGetAsEntry"> & {
  webkitGetAsEntry?: () => WebkitFileSystemEntry | null;
};

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
    entry: WebkitFileSystemEntry,
    parentPath: string,
  ): Promise<UploadFileLike[]> => {
    if (!entry) return [];

    if (entry.isFile && typeof entry.file === "function") {
      const fileEntry = entry.file;
      const file: File = await new Promise((resolve, reject) => {
        fileEntry(
          (f: File) => resolve(f),
          (err: unknown) => reject(err)
        );
      });
      const relativeName = parentPath ? `${parentPath}/${file.name}` : file.name;
      return [
        {
          name: relativeName,
          size: file.size,
          arrayBuffer: () => file.arrayBuffer(),
          slice: (start, end) => file.slice(start, end),
        },
      ];
    }

    if (entry.isDirectory && typeof entry.createReader === "function") {
      const reader = entry.createReader();
      const entries: WebkitFileSystemEntry[] = [];

      await new Promise<void>((resolve, reject) => {
        const readBatch = () => {
          reader.readEntries(
            (batch) => {
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

    const itemWithEntry = item as DataTransferItemWithWebkitEntry;
    if (typeof itemWithEntry.webkitGetAsEntry === "function") {
      const entry = itemWithEntry.webkitGetAsEntry();
      if (entry) {
        entryPromises.push(walkEntry(entry, ""));
        continue;
      }
    }

    const file = item.getAsFile();
    if (file) {
      files.push({
        name: file.name,
        size: file.size,
        arrayBuffer: () => file.arrayBuffer(),
        slice: (start, end) => file.slice(start, end),
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

export interface FileBrowserPaneState {
  sourceId: string;
  storageName: string;
  currentPath: string;
  selectedPaths: string[];
}

export interface FileBrowserPaneTransferTarget {
  sourceId: string;
  storageName: string;
  currentPath: string;
  direction: "left" | "right";
}

interface FileBrowserProps {
  sourceId: string;
  storageName: string;
  refreshTick?: number;
  onPreviewVisibilityChange?: (visible: boolean) => void;
  onToggleSidebar?: () => void;
  isSidebarOpen?: boolean;
  onToggleDualPane?: () => void;
  isDualPane?: boolean;
  showWindowControls?: boolean;
  showTransferQueue?: boolean;
  paneTransferTarget?: FileBrowserPaneTransferTarget | null;
  onPaneStateChange?: (state: FileBrowserPaneState) => void;
  onTransferCompleted?: (storageIds: string[]) => void;
  initialPath?: string;
  headerVariant?: "full" | "pane";
  paneLabel?: string;
}

interface LoadError {
  title: string;
  detail?: string;
}

interface StoredLocation {
  sourceId: string;
  storageName: string;
  path: string;
  updatedAt: number;
}

const BOOKMARKS_STORAGE_KEY = "infimount:file-bookmarks:v1";
const RECENTS_STORAGE_KEY = "infimount:file-recents:v1";

function locationLabel(path: string) {
  if (!path || path === "/") return "/";
  const trimmed = path.replace(/\/$/, "");
  return trimmed.split("/").filter(Boolean).pop() ?? trimmed;
}

function readStoredLocations(key: string): StoredLocation[] {
  if (typeof window === "undefined") return [];
  try {
    const parsed = JSON.parse(window.localStorage.getItem(key) ?? "[]") as StoredLocation[];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item) =>
        typeof item?.sourceId === "string" &&
        typeof item.storageName === "string" &&
        typeof item.path === "string" &&
        typeof item.updatedAt === "number",
    );
  } catch {
    return [];
  }
}

function writeStoredLocations(key: string, locations: StoredLocation[]) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(key, JSON.stringify(locations));
}

export function FileBrowser({
  sourceId,
  storageName,
  refreshTick = 0,
  onPreviewVisibilityChange,
  onToggleSidebar,
  isSidebarOpen,
  onToggleDualPane,
  isDualPane = false,
  showWindowControls = true,
  showTransferQueue = true,
  paneTransferTarget = null,
  onPaneStateChange,
  onTransferCompleted,
  initialPath = "/",
  headerVariant = "full",
  paneLabel,
}: FileBrowserProps) {
  const { zoom } = useAppZoom();
  const [viewMode, setViewMode] = useState<"grid" | "table">("grid");
  const [searchQuery, setSearchQuery] = useState('');
  const [currentPath, setCurrentPath] = useState<string>(initialPath || "/");
  const [allFiles, setAllFiles] = useState<FileItem[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [history, setHistory] = useState<string[]>(["/"]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [nextPageCursor, setNextPageCursor] = useState<string | null>(null);
  const [listingTruncated, setListingTruncated] = useState(false);
  const [error, setError] = useState<LoadError | null>(null);

  type SortField = "name" | "type" | "modified" | "size";
  type SortDirection = "asc" | "desc";

  const [sortField, setSortField] = useState<SortField>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const [previewFile, setPreviewFile] = useState<FileItem | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [pendingDeleteItems, setPendingDeleteItems] = useState<FileItem[] | null>(null);
  const [deleteProgress, setDeleteProgress] = useState<{
    total: number;
    completed: number;
    currentName: string;
    failed: number;
    failedItems: FileItem[];
    cancelled: boolean;
    done: boolean;
  } | null>(null);
  const [uploadProgress, setUploadProgress] = useState<{
    total: number;
    completed: number;
    currentName: string;
    failed: number;
    cancelled: boolean;
  } | null>(null);
  const deleteCancelRef = useRef(false);
  const uploadCancelRef = useRef(false);
  const activeUploadAbortRef = useRef<AbortController | null>(null);
  const [pasteConflict, setPasteConflict] = useState<{
    fromSourceId: string;
    toSourceId: string;
    paths: string[];
    targetDir: string;
    operation: "copy" | "move";
  } | null>(null);
  const [compareResult, setCompareResult] = useState<{
    targetName: string;
    targetDir: string;
    missingPaths: string[];
    changedPaths: string[];
    sameCount: number;
  } | null>(null);
  const [uploadConflict, setUploadConflict] = useState<{ files: UploadFileLike[] } | null>(null);
  const [editTargetId, setEditTargetId] = useState<string | null>(null);
  const { theme: iconTheme, setTheme: setIconTheme } = useIconTheme();
  const { clipboard, setClipboard, clearClipboard } = useFileClipboard();
  const { enqueueTransfer } = useTransferQueue();
  const [isEditingPath, setIsEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const [createTargetType, setCreateTargetType] = useState<"file" | "folder" | null>(null);
  const [newEntryName, setNewEntryName] = useState("");
  const [bookmarks, setBookmarks] = useState<StoredLocation[]>(() =>
    readStoredLocations(BOOKMARKS_STORAGE_KEY),
  );
  const [recents, setRecents] = useState<StoredLocation[]>(() =>
    readStoredLocations(RECENTS_STORAGE_KEY),
  );
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const loadRequestIdRef = useRef(0);
  const loadMoreInFlightRef = useRef(false);

  const describeLoadError = (err: TauriApiError): LoadError => {
    const shortMessage = (err.message || "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)[0];

    const msg = err.message || "";
    const isGcsCredentialLoadFailure =
      msg.includes("service: gcs")
      && (msg.includes("reqsign::LoadCredential") || msg.includes("metadata.google.internal"));

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
        if (isGcsCredentialLoadFailure) {
          return {
            title: "Missing Google Cloud credentials",
            detail:
              "Add a Service Account JSON to this storage (Edit storage → Google Cloud Storage → Service Account JSON). You can paste the raw JSON. Or configure Application Default Credentials (GOOGLE_APPLICATION_CREDENTIALS).",
          };
        }
        return {
          title: "Could not connect to this storage",
          detail: shortMessage || err.message,
        };
    }
  };

  useEffect(() => {
    onPreviewVisibilityChange?.(!!previewFile);
  }, [previewFile, onPreviewVisibilityChange]);

  useEffect(() => {
    onPaneStateChange?.({
      sourceId,
      storageName,
      currentPath,
      selectedPaths: Array.from(selectedFiles),
    });
  }, [currentPath, onPaneStateChange, selectedFiles, sourceId, storageName]);

  const normalizedSearchQuery = searchQuery.toLowerCase();
  const filteredFiles = useMemo(
    () => allFiles.filter((file) => file.name.toLowerCase().includes(normalizedSearchQuery)),
    [allFiles, normalizedSearchQuery],
  );

  const storageBookmarks = useMemo(
    () => bookmarks.filter((item) => item.sourceId === sourceId),
    [bookmarks, sourceId],
  );
  const storageRecents = useMemo(
    () => recents.filter((item) => item.sourceId === sourceId && item.path !== currentPath),
    [currentPath, recents, sourceId],
  );
  const isCurrentPathBookmarked = useMemo(
    () => storageBookmarks.some((item) => item.path === currentPath),
    [currentPath, storageBookmarks],
  );

  const persistBookmarks = useCallback((next: StoredLocation[]) => {
    setBookmarks(next);
    writeStoredLocations(BOOKMARKS_STORAGE_KEY, next);
  }, []);

  const persistRecents = useCallback((next: StoredLocation[]) => {
    setRecents(next);
    writeStoredLocations(RECENTS_STORAGE_KEY, next);
  }, []);

  const rememberRecent = useCallback(
    (path: string) => {
      const location: StoredLocation = {
        sourceId,
        storageName,
        path,
        updatedAt: Date.now(),
      };
      const next = [
        location,
        ...readStoredLocations(RECENTS_STORAGE_KEY).filter(
          (item) => !(item.sourceId === sourceId && item.path === path),
        ),
      ].slice(0, 40);
      persistRecents(next);
    },
    [persistRecents, sourceId, storageName],
  );

  const toggleBookmark = () => {
    const existing = readStoredLocations(BOOKMARKS_STORAGE_KEY);
    const hasBookmark = existing.some(
      (item) => item.sourceId === sourceId && item.path === currentPath,
    );
    const next = hasBookmark
      ? existing.filter((item) => !(item.sourceId === sourceId && item.path === currentPath))
      : [
          {
            sourceId,
            storageName,
            path: currentPath,
            updatedAt: Date.now(),
          },
          ...existing,
        ].slice(0, 80);
    persistBookmarks(next);
    toast({
      title: hasBookmark ? "Bookmark removed" : "Bookmark added",
      description: `${storageName}: ${currentPath}`,
    });
  };

  const mapEntryToFileItem = (entry: Entry): FileItem => ({
    id: entry.path,
    name: entry.name,
    type: entry.is_dir ? "folder" : "file",
    size: entry.is_dir ? undefined : entry.size,
    modified: entry.modified_at ? new Date(entry.modified_at) : null,
    owner: undefined,
    extension: !entry.is_dir ? entry.name.split(".").pop() : undefined,
  });

  const visiblePageEntries = (entries: Entry[], path: string) =>
    entries.filter((entry) => entry.path !== path && entry.path !== "" && entry.name !== ".");

  const loadFiles = async (path: string) => {
    const requestId = loadRequestIdRef.current + 1;
    loadRequestIdRef.current = requestId;
    loadMoreInFlightRef.current = false;
    setLoading(true);
    setLoadingMore(false);
    setNextPageCursor(null);
    setListingTruncated(false);
    setError(null);
    try {
      const page = await listEntriesPage(sourceId, path, 200, undefined, false);
      if (requestId !== loadRequestIdRef.current) return;
      setAllFiles(visiblePageEntries(page.entries, path).map(mapEntryToFileItem));
      setNextPageCursor(page.nextCursor);
      setListingTruncated(page.truncated);
      setSelectedFiles(new Set());
    } catch (err) {
      if (requestId !== loadRequestIdRef.current) return;

      if (err instanceof TauriApiError) {
        setError(describeLoadError(err));
      } else {
        setError({
          title: "Failed to load files",
          detail: err instanceof Error ? err.message : String(err),
        });
      }
      setAllFiles([]); // Clear files on error to prevent showing stale data
    } finally {
      if (requestId === loadRequestIdRef.current) {
        setLoading(false);
      }
    }
  };

  const loadMoreFiles = async () => {
    const cursor = nextPageCursor;
    if (!cursor || loadMoreInFlightRef.current) return;
    const requestId = loadRequestIdRef.current;
    const requestedPath = currentPath;
    loadMoreInFlightRef.current = true;
    setLoadingMore(true);
    try {
      const page = await listEntriesPage(sourceId, requestedPath, 200, cursor, false);
      if (requestId !== loadRequestIdRef.current || requestedPath !== currentPath) return;
      const additions = visiblePageEntries(page.entries, requestedPath).map(mapEntryToFileItem);
      setAllFiles((previous) => {
        const existing = new Set(previous.map((file) => file.id));
        return [...previous, ...additions.filter((file) => !existing.has(file.id))];
      });
      setNextPageCursor(page.nextCursor);
      setListingTruncated(page.truncated);
    } catch (err) {
      if (requestId !== loadRequestIdRef.current || requestedPath !== currentPath) return;
      toast({
        title: "Could not load more files",
        description: err instanceof Error ? err.message : String(err),
        variant: "destructive",
      });
    } finally {
      if (requestId === loadRequestIdRef.current) {
        loadMoreInFlightRef.current = false;
        setLoadingMore(false);
      }
    }
  };

  useEffect(
    () => () => {
      loadRequestIdRef.current += 1;
      loadMoreInFlightRef.current = false;
    },
    [],
  );

  useEffect(() => {
    loadRequestIdRef.current += 1;
    const nextPath = initialPath || "/";
    setCurrentPath(nextPath);
    setError(null);
    setLoading(false);
    setLoadingMore(false);
    setNextPageCursor(null);
    setListingTruncated(false);
    loadMoreInFlightRef.current = false;
    setAllFiles([]); // Clear files when switching sources
    setSelectedFiles(new Set());
    setHistory([nextPath]);
    setHistoryIndex(0);
    setPreviewFile(null);
    setEditTargetId(null);
    setCreateTargetType(null);
    setNewEntryName("");
  // initialPath is only used when the pane is created or its storage changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceId]);

  useEffect(() => {
    void loadFiles(currentPath);
    rememberRecent(currentPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPath, sourceId, refreshTick]);

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

  const handleSelectFiles = useCallback((fileIds: string[]) => {
    const nextIds = [...new Set(fileIds)];
    setSelectedFiles((prev) => {
      if (prev.size === nextIds.length && nextIds.every((id) => prev.has(id))) {
        return prev;
      }
      return new Set(nextIds);
    });
  }, []);

  const clearSelection = () => setSelectedFiles(new Set());

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

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "f") {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }

      if (key === "n" && event.shiftKey) {
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
        setCreateTargetType("folder");
        setNewEntryName("New Folder");
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const setClipboardFromSelection = (operation: "copy" | "move") => {
    if (selectedFiles.size === 0) return;
    setClipboard({
      operation,
      sourceId,
      paths: Array.from(selectedFiles),
    });
  };

  const queueTransfer = (
    request: TransferJobRequest,
    options: {
      onConflict?: () => void;
      onCompleted?: () => void | Promise<void>;
      successDescription?: string;
    } = {},
  ) => {
    enqueueTransfer(request, {
      onCompleted: async () => {
        await loadFiles(currentPath);
        toast({
          title: request.operation === "copy" ? "Copy completed" : "Move completed",
          description:
            options.successDescription ??
            `${request.paths.length} item${request.paths.length === 1 ? "" : "s"} ${
              request.operation === "copy" ? "copied" : "moved"
            }.`,
        });
        await options.onCompleted?.();
      },
      onFailed: (_job, error) => {
        if (error instanceof TauriApiError && error.code === "ALREADY_EXISTS") {
          options.onConflict?.();
          return;
        }
        toast({
          title: request.operation === "copy" ? "Copy failed" : "Move failed",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      },
    });
  };

  const pasteInto = async (targetDir?: string) => {
    if (!clipboard || clipboard.paths.length === 0) {
      return;
    }

    const destinationDir = targetDir ?? currentPath;
    const request: TransferJobRequest = {
      fromSourceId: clipboard.sourceId,
      toSourceId: sourceId,
      paths: clipboard.paths,
      targetDir: destinationDir,
      operation: clipboard.operation,
      conflictPolicy: "fail",
      destinationName: storageName,
    };

    queueTransfer(request, {
      onConflict: () => {
        setPasteConflict({
          fromSourceId: clipboard.sourceId,
          toSourceId: sourceId,
          paths: clipboard.paths,
          targetDir: destinationDir,
          operation: clipboard.operation,
        });
      },
      onCompleted: () => {
        if (clipboard.operation === "move") {
          clearClipboard();
        }
      },
    });
  };

  const moveIntoFolder = async (paths: string[], folderPath: string) => {
    if (paths.length === 0) return;

    queueTransfer(
      {
        fromSourceId: sourceId,
        toSourceId: sourceId,
        paths,
        targetDir: folderPath,
        operation: "move",
        conflictPolicy: "fail",
        sourceName: storageName,
        destinationName: storageName,
      },
      {
        onConflict: () => {
          setPasteConflict({
            fromSourceId: sourceId,
            toSourceId: sourceId,
            paths,
            targetDir: folderPath,
            operation: "move",
          });
        },
        successDescription: `${paths.length} item${paths.length === 1 ? "" : "s"} moved.`,
      },
    );
  };

  const queuePaneTransfer = (operation: "copy" | "move") => {
    if (!paneTransferTarget || selectedFiles.size === 0) return;

    const paths = Array.from(selectedFiles);
    queueTransfer(
      {
        fromSourceId: sourceId,
        toSourceId: paneTransferTarget.sourceId,
        paths,
        targetDir: paneTransferTarget.currentPath,
        operation,
        conflictPolicy: "fail",
        sourceName: storageName,
        destinationName: paneTransferTarget.storageName,
      },
      {
        onConflict: () => {
          setPasteConflict({
            fromSourceId: sourceId,
            toSourceId: paneTransferTarget.sourceId,
            paths,
            targetDir: paneTransferTarget.currentPath,
            operation,
          });
        },
        onCompleted: () => {
          onTransferCompleted?.([sourceId, paneTransferTarget.sourceId]);
        },
        successDescription: `${paths.length} item${paths.length === 1 ? "" : "s"} ${
          operation === "copy" ? "copied" : "moved"
        } to ${paneTransferTarget.storageName}.`,
      },
    );
  };

  const compareWithPaneTarget = async () => {
    if (!paneTransferTarget) return;

    const relativePath = (base: string, path: string) => {
      const normalizedBase = base === "/" ? "" : base.replace(/\/$/, "");
      return path.replace(normalizedBase, "").replace(/^\//, "");
    };

    try {
      const [sourceEntries, targetEntries] = await Promise.all([
        listEntriesRecursive(sourceId, currentPath),
        listEntriesRecursive(paneTransferTarget.sourceId, paneTransferTarget.currentPath),
      ]);
      const targetByRelativePath = new Map(
        targetEntries.map((entry) => [relativePath(paneTransferTarget.currentPath, entry.path), entry]),
      );
      const missingPaths: string[] = [];
      const changedPaths: string[] = [];
      let sameCount = 0;

      for (const sourceEntry of sourceEntries) {
        if (sourceEntry.is_dir) continue;
        const relative = relativePath(currentPath, sourceEntry.path);
        const targetEntry = targetByRelativePath.get(relative);
        if (!targetEntry) {
          missingPaths.push(sourceEntry.path);
          continue;
        }
        if (targetEntry.is_dir || targetEntry.size !== sourceEntry.size) {
          changedPaths.push(sourceEntry.path);
          continue;
        }
        sameCount += 1;
      }

      setCompareResult({
        targetName: paneTransferTarget.storageName,
        targetDir: paneTransferTarget.currentPath,
        missingPaths,
        changedPaths,
        sameCount,
      });
    } catch (error) {
      toast({
        title: "Compare failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    }
  };

  const copyCompareUpdates = () => {
    if (!paneTransferTarget || !compareResult) return;
    const paths = [...compareResult.missingPaths, ...compareResult.changedPaths];
    if (paths.length === 0) return;
    queueTransfer(
      {
        fromSourceId: sourceId,
        toSourceId: paneTransferTarget.sourceId,
        paths,
        targetDir: compareResult.targetDir,
        operation: "copy",
        conflictPolicy: "overwrite",
        sourceName: storageName,
        destinationName: paneTransferTarget.storageName,
      },
      {
        onCompleted: () => {
          onTransferCompleted?.([sourceId, paneTransferTarget.sourceId]);
        },
        successDescription: `${paths.length} item${paths.length === 1 ? "" : "s"} updated from compare result.`,
      },
    );
    setCompareResult(null);
  };

  const paneTransferLabel = paneTransferTarget
    ? `${paneTransferTarget.direction === "right" ? "right" : "left"} pane`
    : "other pane";

  const composeTargetPath = (name: string) => {
    const basePath = currentPath === "/" ? "" : currentPath.replace(/\/$/, "");
    return `${basePath}/${name}`;
  };

  const openCreateTargetDialog = (type: "file" | "folder") => {
    setCreateTargetType(type);
    setNewEntryName(type === "folder" ? "New Folder" : "new-file.txt");
  };

  const createTarget = async () => {
    if (!createTargetType) return;
    const name = newEntryName.trim();

    if (!name) {
      toast({
        title: "Invalid name",
        description: "Please enter a name.",
        variant: "destructive",
      });
      return;
    }

    if (name.includes("/") || name.includes("\\")) {
      toast({
        title: "Invalid name",
        description: "Name cannot include path separators.",
        variant: "destructive",
      });
      return;
    }

    const path = composeTargetPath(name);

    try {
      if (createTargetType === "folder") {
        await createDirectory(sourceId, path);
        toast({
          title: "Folder created",
          description: `"${name}" was created.`,
        });
      } else {
        await writeFile(sourceId, path, new Uint8Array());
        toast({
          title: "File created",
          description: `"${name}" was created.`,
        });
      }

      await loadFiles(currentPath);
      setCreateTargetType(null);
      setNewEntryName("");
    } catch (error: unknown) {
      toast({
        title: createTargetType === "folder" ? "Failed to create folder" : "Failed to create file",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
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
  // pasteInto and setClipboardFromSelection are intentionally captured from the current render.
  // eslint-disable-next-line react-hooks/exhaustive-deps
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
      setPendingDeleteItems(null);
      setShowDeleteConfirm(true);
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectedFiles]);

  const downloadOne = async (file: FileItem) => {
    if (file.type !== "file") return;
    try {
      const result = await downloadFileToDownloads(sourceId, file.id);
      toast({
        title: "Download complete",
        description: `Saved "${result.fileName}" to Downloads.`,
      });
    } catch (error: unknown) {
      toast({
        title: "Download failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    }
  };

  const runDeleteItems = async (items: FileItem[]) => {
    if (items.length === 0) return;

    let failed = 0;
    const failedItems: FileItem[] = [];
    deleteCancelRef.current = false;
    setDeleteProgress({
      total: items.length,
      completed: 0,
      currentName: items[0]?.name ?? "",
      failed: 0,
      failedItems: [],
      cancelled: false,
      done: false,
    });

    for (const [index, file] of items.entries()) {
      if (deleteCancelRef.current) break;
      setDeleteProgress((current) =>
        current
          ? {
              ...current,
              completed: index,
              currentName: file.name,
              cancelled: deleteCancelRef.current,
            }
          : current,
      );

      try {
        await deletePath(sourceId, file.id);
      } catch (error: unknown) {
        failed += 1;
        failedItems.push(file);
        setDeleteProgress((current) =>
          current
            ? {
                ...current,
                failed,
                failedItems: [...failedItems],
              }
            : current,
        );
        toast({
          title: "Delete failed",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      }

      setDeleteProgress((current) =>
        current
          ? {
              ...current,
              completed: index + 1,
              cancelled: deleteCancelRef.current,
            }
          : current,
      );
    }

    await loadFiles(currentPath);
    const wasCancelled = deleteCancelRef.current;
    if (failed > 0 || wasCancelled) {
      setDeleteProgress((current) =>
        current
          ? {
              ...current,
              failed,
              failedItems: [...failedItems],
              cancelled: wasCancelled,
              done: true,
            }
          : current,
      );
    } else {
      setDeleteProgress(null);
    }

    if (failed === 0 && !wasCancelled) {
      toast({
        title: items.length === 1 ? "Item deleted" : "Items deleted",
        description: `${items.length} item${items.length === 1 ? "" : "s"} removed.`,
        variant: "default",
      });
    } else if (wasCancelled) {
      toast({
        title: "Delete stopped",
        description: "No additional selected items will be deleted.",
        variant: "default",
      });
    } else if (failed < items.length) {
      toast({
        title: "Delete partly completed",
        description: `${items.length - failed} of ${items.length} item${items.length === 1 ? "" : "s"} removed.`,
        variant: "default",
      });
    }
  };

  const requestDeleteOne = (file: FileItem) => {
    setPendingDeleteItems([file]);
    setShowDeleteConfirm(true);
  };

  const handleConfirmedDelete = async () => {
    const toDelete = pendingDeleteItems ?? filteredFiles.filter((f) => selectedFiles.has(f.id));
    setShowDeleteConfirm(false);
    setPendingDeleteItems(null);
    await runDeleteItems(toDelete);
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

  type UploadConflictPolicy = "overwrite" | "skip" | "rename";

  const uploadTargetPath = (fileName: string) => {
    const basePath = currentPath === "/" ? "" : currentPath.replace(/\/$/, "");
    return `${basePath}/${fileName}`;
  };

  const existingUploadPaths = () => new Set(allFiles.map((file) => file.id));

  const splitUploadName = (name: string) => {
    const slashIndex = name.lastIndexOf("/");
    const dir = slashIndex >= 0 ? name.slice(0, slashIndex + 1) : "";
    const leaf = slashIndex >= 0 ? name.slice(slashIndex + 1) : name;
    if (leaf.startsWith(".")) return { dir, stem: leaf, ext: "" };
    const dotIndex = leaf.lastIndexOf(".");
    if (dotIndex <= 0) return { dir, stem: leaf, ext: "" };
    return { dir, stem: leaf.slice(0, dotIndex), ext: leaf.slice(dotIndex) };
  };

  const uniqueUploadName = (name: string, reservedPaths: Set<string>) => {
    const { dir, stem, ext } = splitUploadName(name);
    for (let index = 1; index <= 9999; index += 1) {
      const suffix = index === 1 ? " copy" : ` copy ${index}`;
      const candidateName = `${dir}${stem}${suffix}${ext}`;
      const candidatePath = uploadTargetPath(candidateName);
      if (!reservedPaths.has(candidatePath)) {
        reservedPaths.add(candidatePath);
        return candidateName;
      }
    }
    return name;
  };

  const performUpload = (files: UploadFileLike[], conflictPolicy: UploadConflictPolicy) => {
    void (async () => {
      if (!files.length) return;
      uploadCancelRef.current = false;
      const reservedPaths = existingUploadPaths();
      setUploadProgress({
        total: files.length,
        completed: 0,
        currentName: files[0]?.name ?? "",
        failed: 0,
        cancelled: false,
      });

      let successCount = 0;
      let skippedCount = 0;
      let failedCount = 0;
      for (const [index, file] of files.entries()) {
        if (uploadCancelRef.current) break;
        setUploadProgress((current) =>
          current
            ? {
                ...current,
                completed: index,
                currentName: file.name,
                cancelled: uploadCancelRef.current,
              }
            : current,
        );
        try {
          let targetName = file.name;
          let targetPath = uploadTargetPath(targetName);
          const exists = reservedPaths.has(targetPath);
          if (exists && conflictPolicy === "skip") {
            skippedCount += 1;
            setUploadProgress((current) => current ? { ...current, completed: index + 1 } : current);
            continue;
          }
          if (exists && conflictPolicy === "rename") {
            targetName = uniqueUploadName(file.name, reservedPaths);
            targetPath = uploadTargetPath(targetName);
          } else {
            reservedPaths.add(targetPath);
          }

          const abortController = new AbortController();
          activeUploadAbortRef.current = abortController;
          await uploadFileStreaming(sourceId, targetPath, file, {
            isCancelled: () => uploadCancelRef.current,
            signal: abortController.signal,
          });
          activeUploadAbortRef.current = null;
          successCount += 1;
        } catch (error: unknown) {
          activeUploadAbortRef.current = null;
          if (uploadCancelRef.current || (error instanceof DOMException && error.name === "AbortError")) {
            break;
          }
          failedCount += 1;
          setUploadProgress((current) =>
            current
              ? {
                  ...current,
                  failed: failedCount,
                }
              : current,
          );
          toast({
            title: "Upload failed",
            description: error instanceof Error ? error.message : String(error),
            variant: "destructive",
          });
        }
        setUploadProgress((current) =>
          current
            ? {
                ...current,
                completed: index + 1,
                cancelled: uploadCancelRef.current,
              }
            : current,
        );
      }
      await loadFiles(currentPath);
      setUploadProgress(null);
      if (successCount > 0) {
        toast({
          title: uploadCancelRef.current ? "Upload stopped" : "Upload complete",
          description: `${successCount} file${successCount > 1 ? "s" : ""} uploaded successfully.${skippedCount > 0 ? ` ${skippedCount} skipped.` : ""}`,
        });
      } else if (uploadCancelRef.current) {
        toast({
          title: "Upload cancelled",
          description: "No additional files will be uploaded.",
        });
      } else if (skippedCount > 0) {
        toast({
          title: "Upload skipped",
          description: `${skippedCount} existing file${skippedCount === 1 ? "" : "s"} skipped.`,
        });
      }
    })();
  };

  const handleUpload = (files: UploadFileLike[]) => {
    if (!files.length) return;
    const existingPaths = existingUploadPaths();
    if (files.some((file) => existingPaths.has(uploadTargetPath(file.name)))) {
      setUploadConflict({ files });
      return;
    }
    performUpload(files, "overwrite");
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

  const sortedFiles = useMemo(() => {
    const dir = sortDirection === "asc" ? 1 : -1;
    return [...filteredFiles].sort((a, b) => {
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
  }, [filteredFiles, sortDirection, sortField]);

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

  const isExternalFileDrag = (event: React.DragEvent) => {
    const dt = event.dataTransfer;
    if (!dt) return false;

    // Reliable signal for OS file drags.
    if (dt.files && dt.files.length > 0) return true;

    const items = Array.from(dt.items ?? []);
    return items.some((item) => item.kind === "file");
  };

  const deleteProgressValue = deleteProgress
    ? Math.max(8, Math.round((deleteProgress.completed / deleteProgress.total) * 100))
    : 0;
  const uploadProgressValue = uploadProgress
    ? Math.max(8, Math.round((uploadProgress.completed / uploadProgress.total) * 100))
    : 0;

  return (
    <>
      <div
        className="relative flex h-full bg-background"
        onDragOver={(event: React.DragEvent<HTMLDivElement>) => {
          if (!isExternalFileDrag(event)) return;
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
          if (!isExternalFileDrag(event)) return;
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
          <div className={headerVariant === "pane" ? "border-b bg-background" : "border-b bg-muted/30"} data-tauri-drag-region={headerVariant === "full" ? true : undefined}>
            <div className={headerVariant === "pane" ? "flex items-center gap-2 px-3 py-2" : "flex items-center gap-2 px-4 py-3"} data-tauri-drag-region={headerVariant === "full" ? true : undefined}>
              <div className="flex items-center gap-1 tauri-no-drag">
                {headerVariant === "full" ? (
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
                ) : null}
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

              <div className="flex min-w-0 flex-1 items-center gap-2" data-tauri-drag-region={headerVariant === "full" ? true : undefined}>
                {paneLabel ? (
                  <span className="shrink-0 rounded-md bg-muted px-2 py-1 text-[11px] font-medium text-muted-foreground select-none pointer-events-none">
                    {paneLabel}
                  </span>
                ) : null}
                <span className="truncate text-sm font-medium select-none pointer-events-none">
                  {currentLabel}
                </span>
              </div>

              <div className="flex items-center gap-2 tauri-no-drag">
                <div className="relative w-48">
                  <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    ref={searchInputRef}
                    placeholder="Search..."
                    value={searchQuery}
                    onChange={(event) => setSearchQuery(event.target.value)}
                    className="h-8 bg-background pl-9 border-border focus-visible:ring-1 focus-visible:ring-primary/20 focus-visible:ring-offset-0 focus-visible:border-border shadow-sm"
                  />
                </div>

                <label htmlFor="file-upload">
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                    title="Upload files"
                    aria-label="Upload files"
                  >
                    <Upload className="h-4 w-4" />
                  </Button>
                </label>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  onClick={toggleBookmark}
                  className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                  title={isCurrentPathBookmarked ? "Remove bookmark" : "Bookmark folder"}
                  aria-label={isCurrentPathBookmarked ? "Remove bookmark" : "Bookmark folder"}
                >
                  <Star
                    className={`h-4 w-4 ${
                      isCurrentPathBookmarked ? "fill-current text-amber-600" : ""
                    }`}
                  />
                </Button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                      title="Bookmarks and recent folders"
                      aria-label="Bookmarks and recent folders"
                    >
                      <Clock className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="min-w-[240px]">
                    <DropdownMenuLabel className="font-normal">Bookmarks</DropdownMenuLabel>
                    {storageBookmarks.length === 0 ? (
                      <DropdownMenuItem disabled>No bookmarks for this storage</DropdownMenuItem>
                    ) : (
                      storageBookmarks.slice(0, 8).map((item) => (
                        <DropdownMenuItem key={`bookmark-${item.path}`} onClick={() => handleNavigate(item.path)}>
                          <div className="min-w-0">
                            <div className="truncate text-xs">{locationLabel(item.path)}</div>
                            <div className="truncate text-[11px] text-muted-foreground">{item.path}</div>
                          </div>
                        </DropdownMenuItem>
                      ))
                    )}
                    <DropdownMenuSeparator />
                    <DropdownMenuLabel className="font-normal">Recent folders</DropdownMenuLabel>
                    {storageRecents.length === 0 ? (
                      <DropdownMenuItem disabled>No recent folders yet</DropdownMenuItem>
                    ) : (
                      storageRecents.slice(0, 6).map((item) => (
                        <DropdownMenuItem key={`recent-${item.path}`} onClick={() => handleNavigate(item.path)}>
                          <div className="min-w-0">
                            <div className="truncate text-xs">{locationLabel(item.path)}</div>
                            <div className="truncate text-[11px] text-muted-foreground">{item.path}</div>
                          </div>
                        </DropdownMenuItem>
                      ))
                    )}
                  </DropdownMenuContent>
                </DropdownMenu>
                {paneTransferTarget ? (
                  <div className="flex items-center gap-1 rounded-md border border-border/70 bg-background/80 px-1 py-0.5">
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => void compareWithPaneTarget()}
                      className="h-7 px-2 text-xs text-foreground/75 hover:bg-black/5 dark:hover:bg-white/5"
                      title={`Compare this folder with the ${paneTransferLabel}`}
                    >
                      Compare
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      disabled={selectedFiles.size === 0}
                      onClick={() => queuePaneTransfer("copy")}
                      className="h-7 gap-1.5 px-2 text-xs text-foreground/75 hover:bg-black/5 dark:hover:bg-white/5"
                      title={`Copy selected items to the ${paneTransferLabel}`}
                    >
                      <Copy className="h-3.5 w-3.5" />
                      Copy {paneTransferTarget.direction === "right" ? "→" : "←"}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      disabled={selectedFiles.size === 0}
                      onClick={() => queuePaneTransfer("move")}
                      className="h-7 gap-1.5 px-2 text-xs text-foreground/75 hover:bg-black/5 dark:hover:bg-white/5"
                      title={`Move selected items to the ${paneTransferLabel}`}
                    >
                      <MoveRight className="h-3.5 w-3.5" />
                      Move {paneTransferTarget.direction === "right" ? "→" : "←"}
                    </Button>
                  </div>
                ) : null}
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
                {onToggleDualPane && headerVariant === "full" ? (
                  <Button
                    size="icon"
                    variant="ghost"
                    onClick={onToggleDualPane}
                    className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
                    title={isDualPane ? "Close split pane" : "Open split pane"}
                    aria-label={isDualPane ? "Close split pane" : "Open split pane"}
                  >
                    <PanelRight className="h-4 w-4" />
                  </Button>
                ) : null}
                {showWindowControls && headerVariant === "full" ? (
                  <div className="ml-2 pl-2 border-l border-border/50">
                    <WindowControls />
                  </div>
                ) : null}
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
          <ResizablePanelGroup direction="horizontal" className="flex-1 overflow-hidden">
            <ResizablePanel minSize="30%" defaultSize={previewFile ? "70%" : "100%"}>
                <div className="flex h-full flex-col overflow-hidden relative">
                <ContextMenu>
                  <ContextMenuTrigger asChild>
                    <div
                      className="flex-1 overflow-hidden infimount-zoom-shell bg-white dark:bg-background"
                      data-infimount-zoom-region="true"
                      style={{ "--infimount-zoom": zoom } as CSSProperties}
                    >
                      <div className="infimount-zoom-inner">
                        {loading ? (
                          <div className="flex h-full items-center justify-center">
                            <div className="flex flex-col items-center gap-2">
                              <img
                                src={infinityLoader}
                                alt=""
                                aria-hidden="true"
                                className="h-6 w-6"
                                draggable={false}
                              />
                              <span className="text-muted-foreground">Loading files...</span>
                            </div>
                          </div>
                        ) : !error && sortedFiles.length === 0 ? (
                          <div className="flex h-full items-center justify-center">
                            <div className="flex flex-col items-center gap-1 text-center">
                              <p className="text-sm font-medium text-foreground/80">
                                This folder is empty
                              </p>
                              <p className="text-xs text-muted-foreground">
                                Drop files here to upload, or navigate to another folder.
                              </p>
                            </div>
                          </div>
                        ) : viewMode === "grid" ? (
                          <FileGrid
                            sourceId={sourceId}
                            files={sortedFiles}
                            selectedFiles={selectedFiles}
                            onSelectFile={handleSelectFile}
                            onSelectFiles={handleSelectFiles}
                            onOpenFile={handleOpenFile}
                            onEditFile={handleEditFile}
                            onDownloadFile={handleDownloadFile}
                            onDeleteFile={requestDeleteOne}
                            onCutSelected={() => setClipboardFromSelection("move")}
                            onCopySelected={() => setClipboardFromSelection("copy")}
                            canPaste={!!clipboard}
                            onPaste={(targetDir) => void pasteInto(targetDir)}
                            onMoveToFolder={(paths, folderPath) =>
                              void moveIntoFolder(paths, folderPath)
                            }
                            onClearSelection={clearSelection}
                          />
                        ) : (
                          <FileTable
                            sourceId={sourceId}
                            files={sortedFiles}
                            selectedFiles={selectedFiles}
                            onSelectFile={handleSelectFile}
                            onSelectFiles={handleSelectFiles}
                            onOpenFile={handleOpenFile}
                            onEditFile={handleEditFile}
                            onDownloadFile={handleDownloadFile}
                            onDeleteFile={requestDeleteOne}
                            sortField={sortField}
                            sortDirection={sortDirection}
                            onSortChange={toggleSort}
                            onCutSelected={() => setClipboardFromSelection("move")}
                            onCopySelected={() => setClipboardFromSelection("copy")}
                            canPaste={!!clipboard}
                            onPaste={(targetDir) => void pasteInto(targetDir)}
                            onMoveToFolder={(paths, folderPath) =>
                              void moveIntoFolder(paths, folderPath)
                            }
                            onClearSelection={clearSelection}
                          />
                        )}
                      </div>
                    </div>
                  </ContextMenuTrigger>
                  <ContextMenuContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                    <ContextMenuItem
                      onClick={() => {
                        openCreateTargetDialog("folder");
                      }}
                    >
                      New folder
                      <ContextMenuShortcut>⌘⇧N</ContextMenuShortcut>
                    </ContextMenuItem>
                    <ContextMenuItem
                      onClick={() => {
                        openCreateTargetDialog("file");
                      }}
                    >
                      New file
                    </ContextMenuItem>
                    <ContextMenuSeparator />
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

                {(nextPageCursor || listingTruncated) && !error && (
                  <div
                    className="flex h-10 shrink-0 items-center justify-center gap-3 border-t bg-muted/20 px-3"
                    aria-live="polite"
                  >
                    {nextPageCursor ? (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={loadingMore}
                        onClick={() => void loadMoreFiles()}
                      >
                        {loadingMore ? "Loading more…" : "Load more"}
                      </Button>
                    ) : null}
                    {listingTruncated ? (
                      <span className="text-xs text-muted-foreground">
                        Listing reached the backend safety limit.
                      </span>
                    ) : null}
                  </div>
                )}

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
            </ResizablePanel>

            {previewFile && (
              <>
                <ResizableHandle className="group relative flex w-1 items-center justify-center bg-transparent cursor-col-resize transition-colors focus:outline-none z-10 -ml-0.5">
                  <div className="h-full w-[1px] bg-border group-hover:bg-foreground/50 transition-colors" />
                </ResizableHandle>
                <ResizablePanel defaultSize="30%" minSize="20%" maxSize="60%" className="bg-background/50">
                  <div
                    className="h-full w-full infimount-zoom-shell bg-white dark:bg-background"
                    data-infimount-zoom-region="true"
                    style={{ "--infimount-zoom": zoom } as CSSProperties}
                  >
                    <div className="infimount-zoom-inner">
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
                  </div>
                </ResizablePanel>
              </>
            )}
          </ResizablePanelGroup>
        </div>
        {deleteProgress ? (
          <section
            className="absolute bottom-12 right-4 z-30 w-[360px] max-w-[calc(100%-2rem)] rounded-xl border border-border bg-card text-card-foreground shadow-lg"
            aria-label="Deletion in progress"
            aria-live="polite"
          >
            <div className="flex items-start gap-3 px-3 py-3">
              <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-destructive/10 text-destructive">
                <Trash2 className="h-4 w-4" aria-hidden="true" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center justify-between gap-3">
                  <h2 className="text-sm font-medium leading-tight">
                    {deleteProgress.done
                      ? deleteProgress.failed > 0
                        ? `${deleteProgress.failed} delete failed${deleteProgress.failed === 1 ? "" : "s"}`
                        : "Delete stopped"
                      : `Deleting ${deleteProgress.total} item${deleteProgress.total === 1 ? "" : "s"}`}
                  </h2>
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    {deleteProgress.completed}/{deleteProgress.total}
                  </span>
                </div>
                <p className="mt-1 truncate text-[11px] text-muted-foreground">
                  {deleteProgress.cancelled && !deleteProgress.done ? "Stopping after current item" : `Removing ${deleteProgress.currentName}`}
                  {deleteProgress.failed > 0 ? ` · ${deleteProgress.failed} failed` : ""}
                </p>
                <Progress
                  value={deleteProgressValue}
                  className="mt-2 h-1.5 bg-muted"
                  aria-label="Delete progress"
                />
                <div className="mt-2 flex items-center justify-between gap-3">
                  <p className="text-[11px] text-muted-foreground">
                    {deleteProgress.done
                      ? "Review failed items or dismiss this status."
                      : "Large folders can take a while. Keep this window open until deletion finishes."}
                  </p>
                  {deleteProgress.done ? (
                    <div className="flex shrink-0 items-center gap-1">
                      {deleteProgress.failedItems.length > 0 ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-7 px-2 text-xs"
                          onClick={() => void runDeleteItems(deleteProgress.failedItems)}
                        >
                          Retry failed
                        </Button>
                      ) : null}
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-7 px-2 text-xs"
                        onClick={() => setDeleteProgress(null)}
                      >
                        Dismiss
                      </Button>
                    </div>
                  ) : (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs"
                      onClick={() => {
                        deleteCancelRef.current = true;
                        setDeleteProgress((current) => current ? { ...current, cancelled: true } : current);
                      }}
                    >
                      Cancel remaining
                    </Button>
                  )}
                </div>
              </div>
            </div>
          </section>
        ) : null}
        {uploadProgress ? (
          <section
            className="absolute bottom-12 right-4 z-30 w-[360px] max-w-[calc(100%-2rem)] rounded-xl border border-border bg-card text-card-foreground shadow-lg"
            aria-label="Upload in progress"
            aria-live="polite"
          >
            <div className="flex items-start gap-3 px-3 py-3">
              <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
                <Upload className="h-4 w-4" aria-hidden="true" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center justify-between gap-3">
                  <h2 className="text-sm font-medium leading-tight">
                    Uploading {uploadProgress.total} file{uploadProgress.total === 1 ? "" : "s"}
                  </h2>
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    {uploadProgress.completed}/{uploadProgress.total}
                  </span>
                </div>
                <p className="mt-1 truncate text-[11px] text-muted-foreground">
                  {uploadProgress.cancelled ? "Stopping after current file" : `Writing ${uploadProgress.currentName}`}
                  {uploadProgress.failed > 0 ? ` · ${uploadProgress.failed} failed` : ""}
                </p>
                <Progress
                  value={uploadProgressValue}
                  className="mt-2 h-1.5 bg-muted"
                  aria-label="Upload progress"
                />
                <div className="mt-2 flex items-center justify-between gap-3">
                  <p className="text-[11px] text-muted-foreground">
                    Keep this window open until upload finishes.
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-xs"
                    onClick={() => {
                      uploadCancelRef.current = true;
                      activeUploadAbortRef.current?.abort();
                      setUploadProgress((current) => current ? { ...current, cancelled: true } : current);
                    }}
                  >
                    Cancel remaining
                  </Button>
                </div>
              </div>
            </div>
          </section>
        ) : null}
        {showTransferQueue ? (
          <TransferQueuePanel className={deleteProgress || uploadProgress ? "bottom-44" : undefined} />
        ) : null}
      </div>

      <AlertDialog
        open={!!createTargetType}
        onOpenChange={(open) => {
          if (!open) {
            setCreateTargetType(null);
            setNewEntryName("");
          }
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {createTargetType === "folder" ? "Create new folder" : "Create new file"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {createTargetType === "folder"
                ? "Enter a folder name to create in the current directory."
                : "Enter a file name to create an empty file in the current directory."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <Input
            autoFocus
            value={newEntryName}
            onChange={(event) => setNewEntryName(event.target.value)}
            placeholder={createTargetType === "folder" ? "New Folder" : "new-file.txt"}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void createTarget();
              }
            }}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={(event) => {
                event.preventDefault();
                void createTarget();
              }}
            >
              Create
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={!!uploadConflict}
        onOpenChange={(open) => {
          if (!open) setUploadConflict(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Upload existing files?</AlertDialogTitle>
            <AlertDialogDescription>
              One or more files already exist in <span className="font-medium">{storageName}</span>. Choose how to handle matching names.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter className="flex-col gap-2 sm:flex-row">
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                const files = uploadConflict?.files ?? [];
                setUploadConflict(null);
                performUpload(files, "skip");
              }}
            >
              Discard existing
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                const files = uploadConflict?.files ?? [];
                setUploadConflict(null);
                performUpload(files, "rename");
              }}
            >
              Keep both
            </Button>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={(event) => {
                event.preventDefault();
                const files = uploadConflict?.files ?? [];
                setUploadConflict(null);
                performUpload(files, "overwrite");
              }}
            >
              Overwrite
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={showDeleteConfirm}
        onOpenChange={(open) => {
          setShowDeleteConfirm(open);
          if (!open) setPendingDeleteItems(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {pendingDeleteItems?.length === 1 ? `Delete ${pendingDeleteItems[0].name}?` : "Delete selected items?"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete {pendingDeleteItems?.length === 1 ? "this item" : "the selected files and folders"} from{" "}
              <span className="font-medium">{storageName}</span>. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={!!deleteProgress}
              onClick={() => {
                void handleConfirmedDelete();
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={!!compareResult}
        onOpenChange={(open) => {
          if (!open) setCompareResult(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Compare result</AlertDialogTitle>
            <AlertDialogDescription>
              {compareResult ? (
                <span>
                  Compared this folder with {compareResult.targetName}. {compareResult.missingPaths.length} missing, {compareResult.changedPaths.length} changed, {compareResult.sameCount} unchanged.
                </span>
              ) : null}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Close</AlertDialogCancel>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              disabled={
                !compareResult ||
                compareResult.missingPaths.length + compareResult.changedPaths.length === 0
              }
              onClick={(event) => {
                event.preventDefault();
                copyCompareUpdates();
              }}
            >
              Copy missing and changed
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
              One or more items with the same name already exist in this location. Keep both, overwrite, or discard this transfer.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-muted text-foreground hover:bg-muted/80"
              onClick={() => {
                if (!pasteConflict) return;
                const conflict = pasteConflict;
                queueTransfer(
                  {
                    fromSourceId: conflict.fromSourceId,
                    toSourceId: conflict.toSourceId,
                    paths: conflict.paths,
                    targetDir: conflict.targetDir,
                    operation: conflict.operation,
                    conflictPolicy: "skip",
                    sourceName: storageName,
                    destinationName: storageName,
                  },
                  {
                    successDescription: "Existing items were skipped.",
                    onCompleted: () => {
                      onTransferCompleted?.([conflict.fromSourceId, conflict.toSourceId]);
                      if (
                        clipboard &&
                        clipboard.operation === "move" &&
                        clipboard.sourceId === conflict.fromSourceId &&
                        clipboard.paths.length === conflict.paths.length &&
                        clipboard.paths.every((p) => conflict.paths.includes(p))
                      ) {
                        clearClipboard();
                      }
                    },
                  },
                );
                setPasteConflict(null);
              }}
            >
              Discard
            </AlertDialogAction>
            <AlertDialogAction
              className="bg-background text-foreground hover:bg-muted"
              onClick={() => {
                if (!pasteConflict) return;
                const conflict = pasteConflict;
                queueTransfer(
                  {
                    fromSourceId: conflict.fromSourceId,
                    toSourceId: conflict.toSourceId,
                    paths: conflict.paths,
                    targetDir: conflict.targetDir,
                    operation: conflict.operation,
                    conflictPolicy: "rename",
                    sourceName: storageName,
                    destinationName: storageName,
                  },
                  {
                    successDescription: "Conflicting items were renamed.",
                    onCompleted: () => {
                      onTransferCompleted?.([conflict.fromSourceId, conflict.toSourceId]);
                      if (
                        clipboard &&
                        clipboard.operation === "move" &&
                        clipboard.sourceId === conflict.fromSourceId &&
                        clipboard.paths.length === conflict.paths.length &&
                        clipboard.paths.every((p) => conflict.paths.includes(p))
                      ) {
                        clearClipboard();
                      }
                    },
                  },
                );
                setPasteConflict(null);
              }}
            >
              Keep both
            </AlertDialogAction>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                if (!pasteConflict) return;
                const conflict = pasteConflict;
                queueTransfer(
                  {
                    fromSourceId: conflict.fromSourceId,
                    toSourceId: conflict.toSourceId,
                    paths: conflict.paths,
                    targetDir: conflict.targetDir,
                    operation: conflict.operation,
                    conflictPolicy: "overwrite",
                    sourceName: storageName,
                    destinationName: storageName,
                  },
                  {
                    successDescription: "Existing items were overwritten.",
                    onCompleted: () => {
                      onTransferCompleted?.([conflict.fromSourceId, conflict.toSourceId]);
                      if (
                        clipboard &&
                        clipboard.operation === "move" &&
                        clipboard.sourceId === conflict.fromSourceId &&
                        clipboard.paths.length === conflict.paths.length &&
                        clipboard.paths.every((p) => conflict.paths.includes(p))
                      ) {
                        clearClipboard();
                      }
                    },
                  },
                );
                setPasteConflict(null);
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
