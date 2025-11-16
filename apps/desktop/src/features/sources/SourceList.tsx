import React, { useState, useEffect } from "react";
import type { Source } from "../../types/source";
import { useSourceStore } from "./store";
import {
  listSources,
  addSource as apiAddSource,
  removeSource as apiRemoveSource,
  updateSource as apiUpdateSource,
} from "../../lib/api";
import { useToastStore } from "../../lib/toast";
import { SourceForm } from "./SourceForm";
import { Button } from "../../components/Button";

export const SourceList: React.FC<{
  onSelectSource?: (source: Source) => void;
  selectedSourceId?: string | null;
}> = ({ onSelectSource, selectedSourceId }) => {
  const { sources, setSources } = useSourceStore();
  const [showForm, setShowForm] = useState(false);
  const [editingSource, setEditingSource] = useState<Source | null>(null);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const addToast = useToastStore((s) => s.addToast);

  const reloadSources = async () => {
    try {
      const items = await listSources();
      setSources(items);
    } catch (err: any) {
      addToast(err?.message || String(err), "error");
    }
  };

  useEffect(() => {
    reloadSources();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleDelete = async (sourceId: string) => {
    if (!confirm("Are you sure you want to delete this source?")) return;
    try {
      await apiRemoveSource(sourceId);
      await reloadSources();
      addToast("Source deleted", "success");
    } catch (err: any) {
      addToast(err?.message || String(err), "error");
    }
  };

  const handleAddSource = async (source: Source) => {
    try {
      await apiAddSource(source);
      await reloadSources();
      setShowForm(false);
      setEditingSource(null);
      addToast("Source added", "success");
    } catch (err: any) {
      addToast(err?.message || String(err), "error");
    }
  };

  const handleBrowse = (source: Source) => {
    if (source.kind !== "local") {
      addToast("Only local sources are supported right now. Other backends are coming soon.", "info");
      return;
    }
    onSelectSource?.(source);
  };

  const handleEdit = (source: Source) => {
    setEditingSource(source);
    setShowForm(true);
    setOpenMenuId(null);
  };

  const handleUpdateSource = async (source: Source) => {
    try {
      await apiUpdateSource(source);
      await reloadSources();
      setShowForm(false);
      setEditingSource(null);
      addToast("Source updated", "success");
    } catch (err: any) {
      addToast(err?.message || String(err), "error");
    }
  };

  const toggleMenu = (id: string) => {
    setOpenMenuId((current) => (current === id ? null : id));
  };

  const getSourceIcon = (source: Source) => {
    if (source.kind === "local") return "📂";
    return "☁️";
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-2">
        <div>
          <h2 className="text-xs font-semibold tracking-[0.18em] uppercase text-muted-foreground">
            Locations
          </h2>
        </div>
        {!showForm && (
          <Button
            onClick={() => setShowForm(true)}
            size="sm"
            className="mt-2 w-full justify-center"
          >
            + Add Source
          </Button>
        )}
      </div>

      {showForm && (
        <div className="space-y-3 rounded-lg border border-border/40 bg-background/60 p-3">
          <SourceForm
            onSubmit={editingSource ? handleUpdateSource : handleAddSource}
            initialData={editingSource || undefined}
            isEditing={Boolean(editingSource)}
          />
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setShowForm(false);
              setEditingSource(null);
            }}
            className="w-full"
          >
            Cancel
          </Button>
        </div>
      )}

      {sources.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border/40 bg-background/40 p-6 text-center text-xs">
          <p className="text-muted-foreground">No sources configured. Add one to get started!</p>
        </div>
      ) : (
        <div className="space-y-1">
          {sources.map((source) => (
            <div
              key={source.id}
              className={`relative flex w-full items-center justify-between rounded-md px-2 py-1.5 text-sm transition-colors ${
                selectedSourceId === source.id
                  ? "bg-background/80 text-foreground"
                  : "text-foreground hover:bg-background/60"
              }`}
            >
              <button
                type="button"
                onClick={() => handleBrowse(source)}
                className="flex flex-1 items-center gap-2 truncate text-left"
              >
                <span className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-md bg-card text-base">
                  {getSourceIcon(source)}
                </span>
                <span className="truncate">{source.name}</span>
              </button>
              <div className="relative ml-1">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleMenu(source.id);
                  }}
                  className="flex h-7 w-7 items-center justify-center rounded-md text-xl leading-none text-muted-foreground hover:bg-background/80"
                  aria-label="Source actions"
                >
                  ⋮
                </button>
                {openMenuId === source.id && (
                  <div className="absolute right-0 z-10 mt-1 w-32 rounded-md border border-border/40 bg-card py-1 text-xs shadow-lg">
                    <button
                      type="button"
                      className="flex w-full items-center px-3 py-1.5 text-left text-foreground hover:bg-background/80"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleEdit(source);
                      }}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className="flex w-full items-center px-3 py-1.5 text-left text-foreground hover:bg-background/80"
                      onClick={(e) => {
                        e.stopPropagation();
                        reloadSources();
                        setOpenMenuId(null);
                      }}
                    >
                      Refresh
                    </button>
                    <button
                      type="button"
                      className="flex w-full items-center px-3 py-1.5 text-left text-destructive hover:bg-background/80"
                      onClick={(e) => {
                        e.stopPropagation();
                        setOpenMenuId(null);
                        handleDelete(source.id);
                      }}
                    >
                      Delete
                    </button>
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
