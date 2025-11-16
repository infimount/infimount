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

  const handleSelect = (entry: Entry) => {
    setSelectedPath(entry.path);
    onSelectEntry?.(entry);
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
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-4 py-3 border-b bg-background">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold text-foreground">{sourceName}</h2>
          <nav className="ml-3 text-xs text-muted-foreground flex items-center">
            {getBreadcrumbs().map((b, i) => (
              <React.Fragment key={b.path}>
                <button
                  className="hover:underline text-foreground"
                  onClick={() => handleNavigate(b.path)}
                >
                  {i === 0 ? "root" : b.name}
                </button>
                {i < getBreadcrumbs().length - 1 && (
                  <span className="mx-1 text-muted-foreground">/</span>
                )}
              </React.Fragment>
            ))}
          </nav>
        </div>
        <div className="flex items-center gap-2">
          <div className="relative">
            <input
              type="text"
              placeholder="Search files..."
              className="h-8 w-[200px] rounded-lg border bg-background pl-8 pr-4 text-sm text-foreground placeholder:text-muted-foreground shadow-sm"
              onChange={(e) => {
                const q = e.target.value.toLowerCase();
                if (!q) {
                  setEntries(allEntries);
                  return;
                }
                setEntries(allEntries.filter((x) => x.name.toLowerCase().includes(q)));
              }}
            />
            <svg
              className="absolute left-2.5 top-2 h-3 w-3 text-muted-foreground"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
          </div>
          {onBack && (
            <Button variant="outline" size="sm" onClick={onBack}>
              ← Back
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => loadEntries(currentPath)}
          >
            ↻ Refresh
          </Button>
        </div>
      </div>

      {/* Error */}
      {error && (
        <div className="rounded-lg bg-destructive/10 p-4 text-sm text-destructive m-4">
          {error}
        </div>
      )}

      {/* Loading */}
      {loading && (
        <div className="flex flex-1 items-center justify-center">
          <div className="flex flex-col items-center gap-2">
            <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
            <span className="text-muted-foreground">Loading files...</span>
          </div>
        </div>
      )}

      {/* Entries Grid */}
      {!loading && (
        <div className="flex-1 overflow-y-auto overflow-x-hidden">
          {entries.length === 0 ? (
            <div className="flex flex-1 items-center justify-center p-8">
              <div className="text-center text-muted-foreground">
                <div className="mx-auto mb-3 text-2xl">📂</div>
                <p className="text-sm">No files or folders found</p>
              </div>
            </div>
          ) : (
            <div className="p-4">
              <table className="w-full table-fixed text-sm">
                <thead className="sticky top-0 z-20 bg-background shadow-sm">
                  <tr className="border-b border-border/60">
                    <th className="px-3 py-3 text-center text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                      Name
                    </th>
                    <th className="px-3 py-3 text-right text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                      Size
                    </th>
                    <th className="px-3 py-3 text-right text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                      Modified
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {entries.map((entry) => (
                    <tr
                      key={entry.path}
                      className={clsx(
                        "border-b border-border/50 transition-colors hover:bg-accent/40",
                        entry.is_dir ? "cursor-pointer" : "",
                        selectedPath === entry.path ? "bg-accent/60 text-accent-foreground" : ""
                      )}
                      onDoubleClick={() => handleDoubleClick(entry)}
                      onClick={() => handleSelect(entry)}
                    >
                      <td className="px-3 py-3 text-center">
                        <div className="mx-auto flex max-w-[220px] flex-col items-center gap-2 text-center">
                          <span className="text-2xl">{entry.is_dir ? "📁" : "📄"}</span>
                          <button
                            type="button"
                            className="w-full text-sm font-medium leading-tight text-foreground/90 hover:text-foreground focus:outline-none"
                            onClick={(e) => {
                              e.stopPropagation();
                              handleSelect(entry);
                              if (entry.is_dir) {
                                handleNavigate(entry.path);
                              }
                            }}
                          >
                            <span className="block w-full truncate">{entry.name}</span>
                          </button>
                        </div>
                      </td>
                      <td className="px-3 py-3 text-right text-muted-foreground">
                        {entry.is_dir ? "-" : formatBytes(entry.size)}
                      </td>
                      <td className="px-3 py-3 text-right text-[11px] text-muted-foreground">
                        {entry.modified_at || "-"}
                      </td>
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
