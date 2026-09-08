import { useEffect, useMemo, useRef, useState } from "react";
import { CheckCircle2, Send, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  previewAgentTaskPublication,
  publishAgentTaskOutputs,
  type AgentTaskOutputReviewOutput,
  type AgentTaskPublicationPreview,
  type AgentTaskPublicationRequest,
  type AgentTaskPublicationOutput,
  type AgentTaskPublishConflictPolicy,
} from "@/lib/agentTasks";
import { listStorages } from "@/lib/api";
import type { StorageConfig } from "@/types/storage";

interface AgentTaskPublicationPanelProps {
  review: AgentTaskOutputReviewOutput;
}

const MAX_SELECTED_OUTPUTS = 100;

function storageField<T>(storage: StorageConfig, camel: keyof StorageConfig, snake: string): T | undefined {
  const record = storage as unknown as Record<string, unknown>;
  return (record[camel as string] ?? record[snake]) as T | undefined;
}

function writableStorages(storages: StorageConfig[]): StorageConfig[] {
  return storages.filter((storage) => {
    const enabled = Boolean(storageField<boolean>(storage, "enabled", "enabled"));
    const readOnly = Boolean(storageField<boolean>(storage, "readOnly", "read_only"));
    return enabled && !readOnly;
  });
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

export function AgentTaskPublicationPanel({ review }: AgentTaskPublicationPanelProps) {
  const [storages, setStorages] = useState<StorageConfig[]>([]);
  const [loadingStorages, setLoadingStorages] = useState(false);
  const [storageLoadError, setStorageLoadError] = useState(false);
  const [destinationStorageId, setDestinationStorageId] = useState("");
  const [destinationDir, setDestinationDir] = useState("");
  const [conflictPolicy, setConflictPolicy] = useState<AgentTaskPublishConflictPolicy>("fail");
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [preview, setPreview] = useState<AgentTaskPublicationPreview | null>(null);
  const [previewRequestKey, setPreviewRequestKey] = useState<string | null>(null);
  const [previewRunning, setPreviewRunning] = useState(false);
  const [publishRunning, setPublishRunning] = useState(false);
  const [published, setPublished] = useState<AgentTaskPublicationOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const previewGeneration = useRef(0);
  const publishGeneration = useRef(0);

  useEffect(() => {
    previewGeneration.current += 1;
    publishGeneration.current += 1;
    setSelectedPaths(new Set());
    setPreview(null);
    setPreviewRequestKey(null);
    setPreviewRunning(false);
    setPublishRunning(false);
    setPublished(null);
    setError(null);
    setDestinationStorageId("");
    setDestinationDir("");
    setConflictPolicy("fail");
    setStorageLoadError(false);
    setLoadingStorages(true);
    let active = true;
    void listStorages()
      .then((records) => {
        if (!active) return;
        const writable = writableStorages(records);
        setStorages(writable);
        setDestinationStorageId(writable[0]?.id ?? "");
      })
      .catch(() => {
        if (!active) return;
        setStorages([]);
        setDestinationStorageId("");
        setStorageLoadError(true);
      })
      .finally(() => {
        if (active) setLoadingStorages(false);
      });
    return () => {
      active = false;
    };
  }, [review]);

  const request = useMemo<AgentTaskPublicationRequest | null>(() => {
    if (!destinationStorageId || selectedPaths.size === 0) return null;
    const outputs = review.files
      .filter((file) => selectedPaths.has(file.taskPath))
      .map((file) => ({
        taskPath: file.taskPath,
        byteSize: file.byteSize,
        sha256: file.sha256,
      }));
    if (outputs.length === 0 || outputs.length > MAX_SELECTED_OUTPUTS) return null;
    return {
      workspaceId: review.workspaceId,
      taskId: review.taskId,
      outputs,
      destinationStorageId,
      destinationDir: destinationDir.trim(),
      conflictPolicy,
    };
  }, [conflictPolicy, destinationDir, destinationStorageId, review, selectedPaths]);

  const requestKey = useMemo(() => (request ? JSON.stringify(request) : null), [request]);
  const currentRequestKey = useRef(requestKey);
  currentRequestKey.current = requestKey;
  const reviewedPreview =
    preview && previewRequestKey && previewRequestKey === requestKey ? preview : null;

  const invalidatePreview = () => {
    previewGeneration.current += 1;
    setPreviewRunning(false);
    setPreview(null);
    setPreviewRequestKey(null);
    setPublished(null);
    setError(null);
  };

  const toggleOutput = (taskPath: string, checked: boolean) => {
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (checked) {
        if (next.size < MAX_SELECTED_OUTPUTS) next.add(taskPath);
      } else {
        next.delete(taskPath);
      }
      return next;
    });
    invalidatePreview();
  };

  const runPreview = async () => {
    if (!request || !requestKey || previewRunning || publishRunning) return;
    const startedFor = requestKey;
    const reviewedRequest = request;
    const generation = ++previewGeneration.current;
    setPreviewRunning(true);
    setPreview(null);
    setPreviewRequestKey(null);
    setPublished(null);
    setError(null);
    try {
      const result = await previewAgentTaskPublication(reviewedRequest);
      if (previewGeneration.current !== generation || currentRequestKey.current !== startedFor) return;
      setPreview(result);
      setPreviewRequestKey(startedFor);
    } catch (value) {
      if (previewGeneration.current !== generation || currentRequestKey.current !== startedFor) return;
      setError(value instanceof Error ? value.message : "Publication preview failed.");
    } finally {
      if (previewGeneration.current === generation) setPreviewRunning(false);
    }
  };

  const publish = async () => {
    if (!request || !requestKey || !reviewedPreview?.canPublish || publishRunning) return;
    const approvedRequest = request;
    const approvedKey = requestKey;
    const generation = ++publishGeneration.current;
    setPublishRunning(true);
    setError(null);
    try {
      const result = await publishAgentTaskOutputs({
        publication: approvedRequest,
        previewToken: reviewedPreview.previewToken,
      });
      if (publishGeneration.current !== generation || currentRequestKey.current !== approvedKey) return;
      setPublished(result);
      setPreview(null);
      setPreviewRequestKey(null);
    } catch (value) {
      if (publishGeneration.current !== generation || currentRequestKey.current !== approvedKey) return;
      setPublished(null);
      setError(value instanceof Error ? value.message : "Agent Task publication failed.");
    } finally {
      if (publishGeneration.current === generation) setPublishRunning(false);
    }
  };

  return (
    <section className="space-y-4 rounded-xl border border-primary/20 bg-primary/5 p-4" data-testid="agent-task-publication">
      <div>
        <div className="flex items-center gap-2 font-medium">
          <Send className="h-4 w-4 text-primary" />
          Publish approved outputs
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          Select reviewed files explicitly. Publication is copy-only and never overwrites an existing destination; conflicts either stop the plan or get a new name.
        </p>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
          <span>{selectedPaths.size} of {review.files.length} selected</span>
          <span>Maximum {MAX_SELECTED_OUTPUTS} files per publication</span>
        </div>
        <div className="max-h-52 space-y-1 overflow-auto rounded-lg border bg-background p-2">
          {review.files.map((file) => {
            const checked = selectedPaths.has(file.taskPath);
            const atLimit = !checked && selectedPaths.size >= MAX_SELECTED_OUTPUTS;
            return (
              <label key={`${file.taskPath}:${file.sha256}`} className="flex cursor-pointer items-start gap-2 rounded-md px-2 py-2 hover:bg-muted/50">
                <Checkbox
                  checked={checked}
                  disabled={publishRunning || atLimit}
                  onCheckedChange={(value) => toggleOutput(file.taskPath, value === true)}
                  aria-label={`Publish ${file.taskPath}`}
                />
                <span className="min-w-0 flex-1">
                  <span className="block break-all font-mono text-xs">{file.taskPath}</span>
                  <span className="mt-0.5 block text-[11px] text-muted-foreground">{formatBytes(file.byteSize)} · SHA-256 {file.sha256.slice(0, 12)}…</span>
                </span>
              </label>
            );
          })}
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-2">
          <label className="text-sm font-medium">Destination storage</label>
          {loadingStorages ? (
            <div className="rounded-md border px-3 py-2 text-sm text-muted-foreground">Loading writable storages…</div>
          ) : storageLoadError ? (
            <div role="alert" className="rounded-md border border-destructive/30 px-3 py-2 text-sm text-destructive">Writable storages could not be loaded.</div>
          ) : storages.length === 0 ? (
            <div className="rounded-md border border-amber-500/30 px-3 py-2 text-sm text-amber-800 dark:text-amber-300">Add or enable a writable destination storage first.</div>
          ) : (
            <Select
              value={destinationStorageId}
              disabled={publishRunning}
              onValueChange={(value) => {
                setDestinationStorageId(value);
                invalidatePreview();
              }}
            >
              <SelectTrigger aria-label="Publication destination storage">
                <SelectValue placeholder="Choose storage" />
              </SelectTrigger>
              <SelectContent>
                {storages.map((storage) => (
                  <SelectItem key={storage.id} value={storage.id}>{storage.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>

        <div className="space-y-2">
          <label htmlFor="agent-task-publication-dir" className="text-sm font-medium">Destination folder</label>
          <Input
            id="agent-task-publication-dir"
            value={destinationDir}
            disabled={publishRunning}
            placeholder="Leave empty for storage root"
            onChange={(event) => {
              setDestinationDir(event.target.value);
              invalidatePreview();
            }}
          />
        </div>

        <div className="space-y-2 sm:col-span-2">
          <label className="text-sm font-medium">If a destination exists</label>
          <Select
            value={conflictPolicy}
            disabled={publishRunning}
            onValueChange={(value) => {
              setConflictPolicy(value as AgentTaskPublishConflictPolicy);
              invalidatePreview();
            }}
          >
            <SelectTrigger aria-label="Publication conflict policy">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fail">Fail — publish nothing over that path</SelectItem>
              <SelectItem value="rename">Keep both — choose a new destination name</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {reviewedPreview ? (
        <div className="space-y-3 rounded-lg border bg-background p-3" data-testid="agent-task-publication-preview">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="text-sm font-medium">Publication plan</div>
            <div className="text-xs text-muted-foreground">{reviewedPreview.fileCount} files · {formatBytes(reviewedPreview.totalBytes)}</div>
          </div>
          <div className="grid grid-cols-3 gap-2 text-xs">
            <div><span className="text-muted-foreground">Create</span><div className="font-medium">{reviewedPreview.createCount}</div></div>
            <div><span className="text-muted-foreground">Rename</span><div className="font-medium">{reviewedPreview.renameCount}</div></div>
            <div><span className="text-muted-foreground">Conflicts</span><div className="font-medium">{reviewedPreview.conflictCount}</div></div>
          </div>
          <div className="max-h-48 space-y-1 overflow-auto">
            {reviewedPreview.files.map((file) => (
              <div key={`${file.taskPath}:${file.destinationPath}`} className="grid gap-1 rounded-md bg-muted/30 px-2 py-2 text-xs sm:grid-cols-[1fr_auto_1fr] sm:items-center">
                <code className="break-all">{file.taskPath}</code>
                <span className="text-muted-foreground">→ {file.action}</span>
                <code className="break-all sm:text-right">{file.destinationPath}</code>
              </div>
            ))}
          </div>
          {!reviewedPreview.canPublish ? (
            <div className="flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-2 text-xs text-amber-800 dark:text-amber-300">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              <span>Resolve destination conflicts or choose “Keep both” and review the plan again. Nothing has been copied.</span>
            </div>
          ) : null}
        </div>
      ) : null}

      {published ? (
        <div className="rounded-lg border border-green-500/30 bg-green-500/5 p-3 text-sm text-green-800 dark:text-green-300" data-testid="agent-task-publication-success">
          <div className="flex items-center gap-2 font-medium"><CheckCircle2 className="h-4 w-4" />Published {published.files.length} approved output{published.files.length === 1 ? "" : "s"}</div>
          <div className="mt-1 text-xs">Receipt: <code>{published.receiptPath}</code></div>
        </div>
      ) : null}

      {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}

      <div className="flex flex-wrap justify-end gap-2">
        <Button
          variant="outline"
          onClick={() => void runPreview()}
          disabled={!request || previewRunning || publishRunning || loadingStorages}
        >
          {previewRunning ? "Reviewing publication…" : "Review publication"}
        </Button>
        {reviewedPreview?.canPublish ? (
          <Button onClick={() => void publish()} disabled={publishRunning || previewRunning}>
            {publishRunning ? "Publishing…" : `Publish ${reviewedPreview.fileCount} approved output${reviewedPreview.fileCount === 1 ? "" : "s"}`}
          </Button>
        ) : null}
      </div>
    </section>
  );
}
