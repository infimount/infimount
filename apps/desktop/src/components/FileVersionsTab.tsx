import { FileVersion, listVersions, deleteFileVersion } from "@/lib/api";
import { formatDistanceToNow } from "date-fns";
import { Download, Trash2, Clock } from "lucide-react";
import { Button } from "@/components/ui/button";
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
import { useState, useEffect } from "react";
import infinityLoader from "@/assets/loading-infinity.apng";

interface FileVersionsTabProps {
  sourceId: string;
  path: string;
  onVersionDownload: (version: string) => void;
}

export function FileVersionsTab({ sourceId, path, onVersionDownload }: FileVersionsTabProps) {
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [pendingDeleteVersion, setPendingDeleteVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadVersions() {
      setLoading(true);
      setError(null);
      try {
        const res = await listVersions(sourceId, path);
        if (!cancelled) {
          setVersions(res.versions || []);
        }
      } catch (err: any) {
        if (!cancelled) {
          setError(err.message || "Failed to load versions");
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void loadVersions();
    return () => {
      cancelled = true;
    };
  }, [sourceId, path]);

  const confirmDeleteVersion = async () => {
    if (!pendingDeleteVersion) return;

    const version = pendingDeleteVersion;
    setDeleting(version);
    try {
      await deleteFileVersion(sourceId, path, version);
      setVersions((prev) => prev.filter((v) => v.version !== version));
      toast({
        title: "Version deleted",
        description: "The file version was successfully deleted.",
      });
    } catch (err: unknown) {
      toast({
        title: "Failed to delete",
        description: err instanceof Error ? err.message : String(err),
        variant: "destructive",
      });
    } finally {
      setDeleting(null);
      setPendingDeleteVersion(null);
    }
  };

  if (loading) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-xs text-muted-foreground">
        <img src={infinityLoader} alt="" className="h-5 w-5" />
        <span>Loading versions…</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center p-8 text-xs text-destructive">
        <p>{error}</p>
        <p className="mt-2 text-muted-foreground">
          This storage backend may not support versioning, or versioning is not enabled.
        </p>
      </div>
    );
  }

  if (versions.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center p-8 text-xs text-muted-foreground">
        <Clock className="mb-2 h-8 w-8 opacity-20" />
        <p>No previous versions found.</p>
      </div>
    );
  }

  return (
    <>
      <div className="flex flex-col space-y-2 p-4">
        {versions.map((v) => (
          <div
            key={v.version}
            className="flex items-center justify-between rounded-md border p-3 text-sm"
          >
            <div className="flex flex-col gap-1 overflow-hidden">
              <div className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
                <span className="truncate">{v.version}</span>
              </div>
              <div className="text-xs text-muted-foreground">
                {v.modified_at
                  ? formatDistanceToNow(new Date(v.modified_at), { addSuffix: true })
                  : "Unknown date"}
                {v.size_bytes !== null && ` • ${(v.size_bytes / 1024).toFixed(1)} KB`}
              </div>
            </div>
            <div className="flex gap-2 shrink-0">
              <Button
                variant="ghost"
                size="icon"
                title="Download version"
                aria-label="Download version"
                onClick={() => onVersionDownload(v.version)}
              >
                <Download className="h-4 w-4" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                title="Delete version"
                aria-label="Delete version"
                disabled={deleting === v.version}
                onClick={() => setPendingDeleteVersion(v.version)}
                className="text-destructive hover:text-destructive hover:bg-destructive/10"
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </div>
        ))}
      </div>

      <AlertDialog
        open={!!pendingDeleteVersion}
        onOpenChange={(open) => {
          if (!open) setPendingDeleteVersion(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this file version?</AlertDialogTitle>
            <AlertDialogDescription>
              This deletes version <span className="font-mono">{pendingDeleteVersion}</span>. The
              latest file remains, but this version cannot be recovered from Infimount.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={!!deleting}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={!!deleting}
              onClick={() => {
                void confirmDeleteVersion();
              }}
            >
              {deleting ? "Deleting..." : "Delete Version"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
