import React, { useState, useEffect } from "react";
import { Entry, listEntries, TauriApiError } from "../../lib/api";
import { Button } from "../../components/Button";
import clsx from "clsx";

export const FileBrowser: React.FC<{
  sourceId: string;
  sourceName: string;
  onBack?: () => void;
  onSelectEntry?: (entry: Entry) => void;
}> = ({ sourceId, sourceName, onBack, onSelectEntry }) => {
  const [currentPath, setCurrentPath] = useState("/");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [allEntries, setAllEntries] = useState<Entry[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset path when switching sources so each source starts
  // from its own root instead of reusing the previous path.
  useEffect(() => {
    setCurrentPath("/");
  }, [sourceId]);

  useEffect(() => {
    loadEntries(currentPath);
  }, [currentPath, sourceId]);

  const loadEntries = async (path: string) => {
    setLoading(true);
    setError(null);
    try {
    const data = await listEntries(sourceId, path);
    setAllEntries(data);
    setEntries(data);
    } catch (err) {
      if (err instanceof TauriApiError) {
        setError(err.message);
      } else {
        setError("Failed to load entries");
      }
    } finally {
      setLoading(false);
    }
  };

  const handleNavigate = (path: string) => {
    setCurrentPath(path);
  };

  const handleDoubleClick = (entry: Entry) => {
    if (entry.is_dir) {
      handleNavigate(entry.path);
    }
  };

  const getBreadcrumbs = () => {
    const parts = currentPath.split("/").filter(Boolean);
    const items: { name: string; path: string }[] = [{ name: sourceName, path: "/" }];
    let acc = "";
    for (const part of parts) {
      acc += "/" + part;
      items.push({ name: part, path: acc });
    }
    return items;
  };

  return (
    <div className="space-y-4">
      {/* Toolbar */}
      <div className="flex flex-col gap-2 rounded-t-xl border-b border-border/60 bg-card px-3 py-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold text-foreground">{sourceName}</h2>
          <nav className="ml-2 text-xs text-muted-foreground">
            {getBreadcrumbs().map((b, i) => (
              <button
                key={b.path}
                className="underline-offset-2 hover:underline text-muted-foreground mr-1"
                onClick={() => handleNavigate(b.path)}
              >
                {i === 0 ? "root" : b.name}
                {i < getBreadcrumbs().length - 1 && " / "}
              </button>
            ))}
          </nav>
        </div>
            <div className="flex gap-2">
          <input
            type="text"
            placeholder="Search files..."
            className="rounded-md border border-input bg-input px-3 py-1 text-sm text-foreground placeholder:text-muted-foreground"
            onChange={(e) => {
              const q = e.target.value.toLowerCase();
              if (!q) {
                setEntries(allEntries);
                return;
              }
              setEntries(allEntries.filter((x) => x.name.toLowerCase().includes(q)));
            }}
          />
          {onBack && (
            <Button variant="outline" size="sm" onClick={onBack}>
              ← Back
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            className="border-border/40 bg-background/60"
            onClick={() => loadEntries(currentPath)}
          >
            ↻ Refresh
          </Button>
        </div>
      </div>

      {/* Error */}
      {error && (
        <div className="rounded-lg border border-destructive bg-destructive/10 p-4 text-sm text-destructive">
          {error}
        </div>
      )}

      {/* Loading */}
      {loading && (
        <div className="flex items-center justify-center gap-2 rounded-lg border border-border bg-muted/30 p-8">
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
          <span className="text-muted-foreground">Loading...</span>
        </div>
      )}

      {/* Entries Grid */}
      {!loading && (
        <div className="rounded-b-xl border border-border/50 bg-card shadow-sm p-4">
          {entries.length === 0 ? (
            <div className="p-8 text-center text-sm text-muted-foreground">No entries found</div>
          ) : (
            <div>
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border/40 bg-card/80">
                    <th className="px-3 py-2 text-left font-medium text-muted-foreground">Name</th>
                    <th className="px-3 py-2 text-left font-medium text-muted-foreground">Type</th>
                    <th className="px-3 py-2 text-left font-medium text-muted-foreground">Size</th>
                    <th className="px-3 py-2 text-left font-medium text-muted-foreground">Modified</th>
                  </tr>
                </thead>
                <tbody>
                  {entries.map((entry) => (
                    <tr
                      key={entry.path}
                      className={clsx(
                        "border-b border-border/20 transition-colors last:border-b-0",
                        entry.is_dir ? "cursor-pointer hover:bg-muted/20" : "hover:bg-muted/10",
                        selectedPath === entry.path ? "bg-muted/10" : ""
                      )}
                      onDoubleClick={() => handleDoubleClick(entry)}
                      onClick={() => {
                        setSelectedPath(entry.path);
                        onSelectEntry?.(entry);
                      }}
                    >
                      <td className="px-3 py-2">
                        <div className="flex items-center gap-2">
                          <span className="text-lg">{entry.is_dir ? "📁" : "📄"}</span>
                          <button
                            className="font-medium text-left hover:underline"
                            onClick={(e) => {
                              e.stopPropagation();
                              if (entry.is_dir) handleNavigate(entry.path);
                              else setSelectedPath(entry.path);
                              onSelectEntry?.(entry);
                            }}
                          >
                            {entry.name}
                          </button>
                        </div>
                      </td>
                      <td className="px-3 py-2 text-muted-foreground">{entry.is_dir ? "Folder" : "File"}</td>
                      <td className="px-3 py-2 text-muted-foreground">{entry.is_dir ? "-" : formatBytes(entry.size)}</td>
                      <td className="px-3 py-2 text-[11px] text-muted-foreground">{entry.modified_at || "-"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return Math.round((bytes / Math.pow(k, i)) * 100) / 100 + " " + sizes[i];
}
