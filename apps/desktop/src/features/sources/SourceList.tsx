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
    <>
      <div className="mb-3">
        {!showForm && (
          <Button
            onClick={() => setShowForm(true)}
            className="w-full justify-center"
            size="sm"
          >
            + Add Source
          </Button>
        )}
      </div>

      {showForm && (
        <div className="mb-3 rounded-lg border bg-background p-3 shadow-sm">
          <SourceForm
            onSubmit={editingSource ? handleUpdateSource : handleAddSource}
            initialData={editingSource || undefined}
            isEditing={Boolean(editingSource)}
          />
          <div className="flex gap-2 mt-2">
            <Button
              variant="secondary"
              size="sm"
              className="flex-1"
              onClick={() => {
                setShowForm(false);
                setEditingSource(null);
              }}
            >
              Cancel
            </Button>
            <Button
              size="sm"
              className="flex-1"
              form="source-form"
              type="submit"
            >
              {editingSource ? "Update" : "Add"}
            </Button>
          </div>
        </div>
      )}

      {sources.length === 0 ? (
        <div className="text-center py-8">
          <p className="text-sm text-muted-foreground">
            No sources added yet
          </p>
        </div>
      ) : (
        <div className="space-y-1">
          {sources.map((source) => (
            <div
              key={source.id}
              className={`group relative flex w-full items-center justify-between rounded-lg px-3 py-2 text-sm transition-colors ${
                selectedSourceId === source.id
                  ? "bg-primary/10 text-primary"
                  : "text-foreground hover:bg-accent"
              }`}
            >
              <button
                type="button"
                onClick={() => handleBrowse(source)}
                className="flex flex-1 items-center gap-2 truncate text-left"
              >
                <span className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md bg-primary/10 text-base">
                  {getSourceIcon(source)}
                </span>
                <span className="truncate font-medium">{source.name}</span>
              </button>
              <div className="opacity-0 group-hover:opacity-100 transition-opacity">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleMenu(source.id);
                  }}
                  className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
                  aria-label="Source actions"
                >
                  ⋯
                </button>
                {openMenuId === source.id && (
                  <div className="absolute right-0 z-10 mt-1 w-32 rounded-lg border bg-popover p-1 text-sm shadow-md">
                    <button
                      type="button"
                      className="flex w-full items-center px-3 py-1.5 text-left text-foreground hover:bg-accent rounded-md"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleEdit(source);
                      }}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className="flex w-full items-center px-3 py-1.5 text-left text-foreground hover:bg-accent rounded-md"
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
                      className="flex w-full items-center px-3 py-1.5 text-left text-destructive hover:bg-destructive/10 rounded-md"
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
    </>
  );
};
