import { useEffect, useMemo, useRef, useState } from "react";
import { Database, RefreshCw, Search, Square } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { listEntriesRecursive, type Entry } from "@/lib/api";
import type { StorageConfig } from "@/types/storage";
import { toast } from "@/hooks/use-toast";

interface IndexedStorage {
  storageId: string;
  storageName: string;
  indexedAt: number;
  entries: Entry[];
}

interface GlobalSearchDialogProps {
  open: boolean;
  storages: StorageConfig[];
  onOpenChange: (open: boolean) => void;
  onSelectStorage?: (storageId: string) => void;
}

const STORAGE_INDEX_KEY = "infimount:storage-index:v1";

function readIndex(): Record<string, IndexedStorage> {
  if (typeof window === "undefined") return {};
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_INDEX_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return parsed as Record<string, IndexedStorage>;
  } catch {
    return {};
  }
}

function writeIndex(index: Record<string, IndexedStorage>) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(STORAGE_INDEX_KEY, JSON.stringify(index));
}

function formatIndexAge(indexedAt: number) {
  const diffMinutes = Math.max(0, Math.round((Date.now() - indexedAt) / 60_000));
  if (diffMinutes < 1) return "just now";
  if (diffMinutes < 60) return `${diffMinutes}m ago`;
  return `${Math.round(diffMinutes / 60)}h ago`;
}

export function GlobalSearchDialog({
  open,
  storages,
  onOpenChange,
  onSelectStorage,
}: GlobalSearchDialogProps) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState<Record<string, IndexedStorage>>(() => readIndex());
  const [indexingStorageId, setIndexingStorageId] = useState<string | null>(null);
  const activeIndexRequestIdRef = useRef(0);

  useEffect(() => {
    if (!open) {
      activeIndexRequestIdRef.current += 1;
      setIndexingStorageId(null);
    }
  }, [open]);

  useEffect(
    () => () => {
      activeIndexRequestIdRef.current += 1;
    },
    [],
  );

  const indexedStorages = storages.map((storage) => ({
    storage,
    indexed: index[storage.id],
  }));

  const results = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return [];
    return Object.values(index)
      .flatMap((indexed) =>
        indexed.entries.map((entry) => ({
          storageId: indexed.storageId,
          storageName: indexed.storageName,
          entry,
        })),
      )
      .filter(({ entry }) =>
        `${entry.name} ${entry.path}`.toLowerCase().includes(normalized),
      )
      .slice(0, 100);
  }, [index, query]);

  const indexStorage = async (storage: StorageConfig) => {
    const requestId = activeIndexRequestIdRef.current + 1;
    activeIndexRequestIdRef.current = requestId;
    setIndexingStorageId(storage.id);
    try {
      const entries = await listEntriesRecursive(storage.id, "/");
      if (requestId !== activeIndexRequestIdRef.current) return;
      const next = {
        ...readIndex(),
        [storage.id]: {
          storageId: storage.id,
          storageName: storage.name,
          indexedAt: Date.now(),
          entries,
        },
      };
      writeIndex(next);
      setIndex(next);
      toast({
        title: "Storage indexed",
        description: `${storage.name}: ${entries.length} paths indexed locally.`,
      });
    } catch (error) {
      if (requestId !== activeIndexRequestIdRef.current) return;
      toast({
        title: "Index failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      if (requestId === activeIndexRequestIdRef.current) {
        setIndexingStorageId(null);
      }
    }
  };

  const cancelIndexing = () => {
    if (!indexingStorageId) return;
    activeIndexRequestIdRef.current += 1;
    setIndexingStorageId(null);
    toast({
      title: "Indexing cancelled",
      description: "The in-flight storage response will be ignored.",
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[86vh] max-w-3xl overflow-hidden rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-base font-normal">Global search</DialogTitle>
          <DialogDescription className="text-left text-xs">
            Opt in per storage. Infimount stores path, name, size, type, and modified metadata locally.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search indexed paths..."
              className="pl-9"
            />
          </div>

          <div className="grid gap-4 md:grid-cols-[260px_1fr]">
            <div className="rounded-xl border border-border/70 bg-card/40 p-3">
              <div className="mb-2 flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <div className="flex items-center gap-2">
                  <Database className="h-3.5 w-3.5" />
                  Indexed storages
                </div>
                {indexingStorageId ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-[11px]"
                    onClick={cancelIndexing}
                  >
                    <Square className="mr-1.5 h-3 w-3" />
                    Stop
                  </Button>
                ) : null}
              </div>
              <div className="space-y-2">
                {indexedStorages.map(({ storage, indexed }) => (
                  <div
                    key={storage.id}
                    className="rounded-lg border border-border bg-background px-3 py-2"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="truncate text-xs text-foreground">{storage.name}</p>
                        <p className="text-[11px] text-muted-foreground">
                          {indexed
                            ? `${indexed.entries.length} paths, ${formatIndexAge(indexed.indexedAt)}`
                            : "Not indexed"}
                        </p>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        disabled={indexingStorageId !== null}
                        aria-label={indexingStorageId === storage.id ? `Indexing ${storage.name}` : `Index ${storage.name}`}
                        onClick={() => void indexStorage(storage)}
                      >
                        <RefreshCw
                          className={`h-3.5 w-3.5 ${indexingStorageId === storage.id ? "animate-spin" : ""}`}
                        />
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div className="rounded-xl border border-border/70 bg-card/40">
              <div className="border-b border-border/70 px-3 py-2 text-xs text-muted-foreground">
                {query.trim() ? `${results.length} result${results.length === 1 ? "" : "s"}` : "Search results"}
              </div>
              <ScrollArea className="h-[360px]">
                {results.length === 0 ? (
                  <div className="px-4 py-10 text-center text-xs text-muted-foreground">
                    {query.trim()
                      ? "No indexed paths match that search."
                      : "Index a storage, then search by file or folder name."}
                  </div>
                ) : (
                  <div className="divide-y divide-border/70">
                    {results.map(({ storageId, storageName, entry }) => (
                      <button
                        key={`${storageId}:${entry.path}`}
                        type="button"
                        className="block w-full px-3 py-2 text-left hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        onClick={() => {
                          onSelectStorage?.(storageId);
                          onOpenChange(false);
                        }}
                      >
                        <div className="flex items-center justify-between gap-3">
                          <span className="truncate text-sm text-foreground">{entry.name}</span>
                          <span className="shrink-0 rounded-full border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
                            {entry.is_dir ? "folder" : "file"}
                          </span>
                        </div>
                        <p className="mt-1 truncate text-[11px] text-muted-foreground">
                          {storageName} · {entry.path}
                        </p>
                      </button>
                    ))}
                  </div>
                )}
              </ScrollArea>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
