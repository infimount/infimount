import { useEffect, useMemo, useRef, useState } from "react";
import { Bot, CheckCircle2, FileStack, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  prepareAgentTask,
  preflightAgentTask,
  type AgentTaskPreflightOutput,
  type AgentTaskPreparationRequest,
  type PrepareAgentTaskOutput,
} from "@/lib/agentTasks";
import { listStorages, listWorkspaces, type WorkspaceRecord } from "@/lib/api";
import type { StorageConfig } from "@/types/storage";

interface AgentTaskPrepareDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sourceId: string;
  storageName: string;
  selectedPaths: string[];
  onPrepared?: (result: PrepareAgentTaskOutput) => void;
}

interface EligibleWorkspace {
  workspace: WorkspaceRecord;
  storageName: string;
}

function storageField<T>(
  storage: StorageConfig,
  camel: keyof StorageConfig,
  snake: string,
): T | undefined {
  const record = storage as unknown as Record<string, unknown>;
  return (record[camel as string] ?? record[snake]) as T | undefined;
}

function workspaceStorageId(workspace: WorkspaceRecord): string {
  const record = workspace as unknown as Record<string, unknown>;
  return String(workspace.storageId ?? record.storage_id ?? "");
}

function workspaceAccessProfile(workspace: WorkspaceRecord): string {
  const record = workspace as unknown as Record<string, unknown>;
  return String(workspace.accessProfile ?? record.access_profile ?? "read_only");
}

function eligibleWorkspaces(
  workspaces: WorkspaceRecord[],
  storages: StorageConfig[],
): EligibleWorkspace[] {
  const storageById = new Map(
    storages.map((storage) => [
      String(storageField<string>(storage, "id", "id") ?? ""),
      storage,
    ]),
  );

  return workspaces.flatMap((workspace) => {
    const storage = storageById.get(workspaceStorageId(workspace));
    if (!storage) return [];
    const backend = String(storageField<string>(storage, "backend", "backend") ?? "");
    const type = String(storageField<string>(storage, "type", "type") ?? "");
    const enabled = Boolean(storageField<boolean>(storage, "enabled", "enabled"));
    const readOnly = Boolean(storageField<boolean>(storage, "readOnly", "read_only"));
    if (
      workspaceAccessProfile(workspace) !== "read_write" ||
      !enabled ||
      readOnly ||
      !(type === "local-fs" || backend === "local" || backend === "fs")
    ) {
      return [];
    }
    return [
      {
        workspace,
        storageName: String(storageField<string>(storage, "name", "name") ?? "Local"),
      },
    ];
  });
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function defaultTitle(paths: string[]): string {
  if (paths.length === 1) {
    const leaf = paths[0]?.split("/").filter(Boolean).pop();
    return leaf ? `Work with ${leaf}` : "Agent task";
  }
  return `Agent task with ${paths.length} items`;
}

function normalizeRequestedOutputs(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => (line.startsWith("outputs/") ? line : `outputs/${line}`));
}

export function AgentTaskPrepareDialog({
  open,
  onOpenChange,
  sourceId,
  storageName,
  selectedPaths,
  onPrepared,
}: AgentTaskPrepareDialogProps) {
  const [workspaces, setWorkspaces] = useState<EligibleWorkspace[]>([]);
  const [loadingWorkspaces, setLoadingWorkspaces] = useState(false);
  const [workspaceLoadError, setWorkspaceLoadError] = useState(false);
  const [workspaceId, setWorkspaceId] = useState("");
  const [title, setTitle] = useState(() => defaultTitle(selectedPaths));
  const [objective, setObjective] = useState("");
  const [requestedOutputs, setRequestedOutputs] = useState("");
  const [preflight, setPreflight] = useState<AgentTaskPreflightOutput | null>(null);
  const [preflightRequestKey, setPreflightRequestKey] = useState<string | null>(null);
  const [preflightRunning, setPreflightRunning] = useState(false);
  const [prepareRunning, setPrepareRunning] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [prepared, setPrepared] = useState<PrepareAgentTaskOutput | null>(null);

  useEffect(() => {
    if (!open) {
      if (prepared) setPrepared(null);
      return;
    }
    // FileBrowser clears its selection after a successful prepare. Keep the
    // committed result visible until this modal closes instead of treating that
    // parent state change as a new request.
    if (prepared) return;

    setTitle(defaultTitle(selectedPaths));
    setObjective("");
    setRequestedOutputs("");
    setPreflight(null);
    setPreflightRequestKey(null);
    setRequestError(null);
    setWorkspaceLoadError(false);
    setLoadingWorkspaces(true);
    void Promise.all([listWorkspaces(), listStorages()])
      .then(([workspaceRecords, storageRecords]) => {
        const eligible = eligibleWorkspaces(workspaceRecords, storageRecords);
        setWorkspaces(eligible);
        setWorkspaceId((current) =>
          current && eligible.some((item) => item.workspace.id === current)
            ? current
            : (eligible[0]?.workspace.id ?? ""),
        );
      })
      .catch(() => {
        setWorkspaces([]);
        setWorkspaceId("");
        setWorkspaceLoadError(true);
      })
      .finally(() => setLoadingWorkspaces(false));
  }, [open, prepared, selectedPaths]);

  const request = useMemo<AgentTaskPreparationRequest | null>(() => {
    if (!workspaceId || !title.trim() || !objective.trim() || selectedPaths.length === 0) {
      return null;
    }
    return {
      sourceStorageId: sourceId,
      sourcePaths: [...selectedPaths],
      workspaceId,
      title: title.trim(),
      objective: objective.trim(),
      requestedOutputs: normalizeRequestedOutputs(requestedOutputs),
    };
  }, [objective, requestedOutputs, selectedPaths, sourceId, title, workspaceId]);

  const requestKey = useMemo(() => (request ? JSON.stringify(request) : null), [request]);
  const currentRequestKey = useRef<string | null>(requestKey);
  currentRequestKey.current = requestKey;
  const reviewedPreflight =
    preflight && preflightRequestKey && preflightRequestKey === requestKey ? preflight : null;

  const invalidateReview = () => {
    setPreflight(null);
    setPreflightRequestKey(null);
    setPrepared(null);
    setRequestError(null);
  };

  const runPreflight = async () => {
    if (!request || !requestKey || preflightRunning) return;
    const reviewedRequest = request;
    const reviewedRequestKey = requestKey;
    setPreflightRunning(true);
    setPreflight(null);
    setPreflightRequestKey(null);
    setRequestError(null);
    try {
      const result = await preflightAgentTask(reviewedRequest);
      if (currentRequestKey.current !== reviewedRequestKey) return;
      setPreflight(result);
      setPreflightRequestKey(reviewedRequestKey);
    } catch (error) {
      if (currentRequestKey.current !== reviewedRequestKey) return;
      setPreflight(null);
      setPreflightRequestKey(null);
      setRequestError(error instanceof Error ? error.message : "Agent Task preflight failed.");
    } finally {
      setPreflightRunning(false);
    }
  };

  const prepare = async () => {
    if (!request || !reviewedPreflight || prepareRunning) return;
    setPrepareRunning(true);
    setRequestError(null);
    try {
      const result = await prepareAgentTask(request);
      setPrepared(result);
      onPrepared?.(result);
    } catch (error) {
      setPrepared(null);
      setRequestError(error instanceof Error ? error.message : "Agent Task preparation failed.");
    } finally {
      setPrepareRunning(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[680px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Bot className="h-5 w-5 text-primary" />
            Prepare Agent Task
          </DialogTitle>
          <DialogDescription>
            {prepared
              ? `The task is committed in ${prepared.workspaceName}.`
              : `Copy the selected ${selectedPaths.length === 1 ? "item" : `${selectedPaths.length} items`} from ${storageName} into a scoped local Agent Workspace. The source is not moved or newly exposed to MCP.`}
          </DialogDescription>
        </DialogHeader>

        {prepared ? (
          <div className="space-y-4 py-2" data-testid="agent-task-prepared">
            <div className="rounded-xl border border-green-200 bg-green-50 p-4 dark:border-green-900/40 dark:bg-green-950/20">
              <div className="flex items-center gap-2 font-medium text-green-800 dark:text-green-300">
                <CheckCircle2 className="h-5 w-5" />
                Task prepared
              </div>
              <p className="mt-2 text-sm text-green-800/80 dark:text-green-300/80">
                {prepared.preparedFiles} file{prepared.preparedFiles === 1 ? "" : "s"} ({formatBytes(prepared.totalBytes)}) copied into {prepared.workspaceName}.
              </p>
              <code className="mt-3 block break-all rounded-md bg-background/70 px-3 py-2 text-xs">
                {prepared.taskRoot}
              </code>
            </div>
            <p className="text-sm text-muted-foreground">
              Next, use your connected agent with this workspace. Put deliverables under <code>outputs/</code>; Infimount review and publication are added in the following v0.8.1 slices.
            </p>
          </div>
        ) : (
          <div className="space-y-4 py-2">
            <div className="rounded-lg border bg-muted/20 p-3 text-sm">
              <div className="flex items-center gap-2 font-medium">
                <FileStack className="h-4 w-4 text-primary" />
                Selected input
              </div>
              <div className="mt-1 text-muted-foreground">
                {selectedPaths.length} selected {selectedPaths.length === 1 ? "item" : "items"} from {storageName}
              </div>
            </div>

            <div className="grid gap-2">
              <label htmlFor="agent-task-title" className="text-sm font-medium">Task title</label>
              <Input
                id="agent-task-title"
                value={title}
                maxLength={120}
                disabled={prepareRunning}
                onChange={(event) => {
                  setTitle(event.target.value);
                  invalidateReview();
                }}
              />
            </div>

            <div className="grid gap-2">
              <label htmlFor="agent-task-objective" className="text-sm font-medium">What should the agent do?</label>
              <Textarea
                id="agent-task-objective"
                value={objective}
                placeholder="Example: Validate the selected export, identify malformed records, and summarize the findings."
                className="min-h-[110px]"
                disabled={prepareRunning}
                onChange={(event) => {
                  setObjective(event.target.value);
                  invalidateReview();
                }}
              />
            </div>

            <div className="grid gap-2">
              <label htmlFor="agent-task-outputs" className="text-sm font-medium">Expected outputs <span className="font-normal text-muted-foreground">(optional, one per line)</span></label>
              <Textarea
                id="agent-task-outputs"
                value={requestedOutputs}
                placeholder={"summary.md\ninvalid-records.csv"}
                className="min-h-[72px] font-mono text-xs"
                disabled={prepareRunning}
                onChange={(event) => {
                  setRequestedOutputs(event.target.value);
                  invalidateReview();
                }}
              />
            </div>

            <div className="grid gap-2">
              <label className="text-sm font-medium">Local Agent Workspace</label>
              {loadingWorkspaces ? (
                <div className="rounded-md border px-3 py-2 text-sm text-muted-foreground">Loading eligible workspaces…</div>
              ) : workspaceLoadError ? (
                <div role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                  Infimount could not load Agent Workspaces.
                </div>
              ) : workspaces.length === 0 ? (
                <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-3 text-sm text-amber-800 dark:text-amber-300">
                  Create a read-write Agent Workspace on Local Filesystem storage first. Remote and read-only workspaces are intentionally not eligible for Agent Task staging yet.
                </div>
              ) : (
                <Select
                  value={workspaceId}
                  disabled={prepareRunning}
                  onValueChange={(value) => {
                    setWorkspaceId(value);
                    invalidateReview();
                  }}
                >
                  <SelectTrigger aria-label="Local Agent Workspace">
                    <SelectValue placeholder="Choose a workspace" />
                  </SelectTrigger>
                  <SelectContent>
                    {workspaces.map(({ workspace, storageName: localStorageName }) => (
                      <SelectItem key={workspace.id} value={workspace.id}>
                        {workspace.name} · {localStorageName}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>

            {reviewedPreflight ? (
              <div className="space-y-3 rounded-xl border border-primary/20 bg-primary/5 p-4" data-testid="agent-task-preflight">
                <div className="font-medium">Scope review</div>
                <div className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
                  <div><div className="text-xs text-muted-foreground">Selected</div><div>{reviewedPreflight.selectedItems}</div></div>
                  <div><div className="text-xs text-muted-foreground">Files</div><div>{reviewedPreflight.fileCount}</div></div>
                  <div><div className="text-xs text-muted-foreground">Folders</div><div>{reviewedPreflight.directoryCount}</div></div>
                  <div><div className="text-xs text-muted-foreground">Prepared size</div><div>{formatBytes(reviewedPreflight.totalBytes)}</div></div>
                </div>
                <div className="text-xs text-muted-foreground">
                  This checks the selected scope and current size. Prepare rechecks the source before copying. The prepared copy will be written to <strong>{reviewedPreflight.workspaceName}</strong>. Source MCP exposure is {reviewedPreflight.sourceMcpExposed ? "already enabled independently" : "not enabled"}.
                </div>
                {reviewedPreflight.warnings.map((warning) => (
                  <div key={warning} className="flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-2 text-xs text-amber-800 dark:text-amber-300">
                    <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                    <span>{warning}</span>
                  </div>
                ))}
              </div>
            ) : null}

            {requestError ? (
              <p role="alert" className="text-sm text-destructive">{requestError}</p>
            ) : null}
          </div>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={prepareRunning}>
            {prepared ? "Close" : "Cancel"}
          </Button>
          {!prepared && !reviewedPreflight ? (
            <Button
              onClick={() => void runPreflight()}
              disabled={!request || loadingWorkspaces || preflightRunning || prepareRunning}
            >
              {preflightRunning ? "Checking…" : "Review scope"}
            </Button>
          ) : null}
          {!prepared && reviewedPreflight ? (
            <Button onClick={() => void prepare()} disabled={!request || prepareRunning}>
              {prepareRunning ? "Preparing…" : "Prepare task"}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}