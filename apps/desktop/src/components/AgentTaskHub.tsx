import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Bot,
  Check,
  Clipboard,
  FileCheck2,
  FolderPlus,
  Loader2,
  RefreshCw,
  Send,
  ShieldCheck,
  Sparkles,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import {
  createAgentWorkspace,
  defaultWorkspacePath,
  listAgentWorkspaces,
  type AgentWorkspace,
} from "@/lib/agentWorkspaces";
import {
  formatTaskBytes,
  listAgentTaskOutputs,
  listAgentTasks,
  normalizeRequestedOutputs,
  prepareAgentTask,
  preflightAgentTask,
  publishAgentTaskOutputs,
  type AgentTaskOutputEntry,
  type AgentTaskPreflightOutput,
  type AgentTaskSummary,
  type PrepareAgentTaskOutput,
  type PublishAgentTaskResult,
} from "@/lib/agentTasks";
import { listStorages, readFileRange } from "@/lib/api";
import { cn } from "@/lib/utils";

const INTERNAL_TRANSFER_MIME = "application/x-infimount-transfer";
const MAX_TEXT_PREVIEW_BYTES = 128 * 1024;

interface TaskStorage {
  id: string;
  name: string;
  backend: string;
  enabled: boolean;
  readOnly: boolean;
  mcpExposed: boolean;
}

interface SourceSelection {
  sourceId: string;
  paths: string[];
}

type HubMode = "prepare" | "tasks";

function mapTaskStorage(value: unknown): TaskStorage | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const id = typeof raw.id === "string" ? raw.id : "";
  const name = typeof raw.name === "string" ? raw.name : "";
  const backend = typeof raw.backend === "string" ? raw.backend : "";
  if (!id || !name || !backend) return null;
  return {
    id,
    name,
    backend,
    enabled: raw.enabled !== false,
    readOnly:
      typeof raw.readOnly === "boolean"
        ? raw.readOnly
        : typeof raw.read_only === "boolean"
          ? raw.read_only
          : false,
    mcpExposed:
      typeof raw.mcpExposed === "boolean"
        ? raw.mcpExposed
        : typeof raw.mcp_exposed === "boolean"
          ? raw.mcp_exposed
          : false,
  };
}

function isLocalStorage(storage: TaskStorage | undefined | null) {
  return storage?.backend === "local" || storage?.backend === "fs";
}

function joinPath(...parts: string[]) {
  return parts
    .map((part) => part.trim().replace(/^\/+|\/+$/g, ""))
    .filter(Boolean)
    .join("/");
}

function basename(path: string) {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.split("/").filter(Boolean).pop() ?? "selected files";
}

function parseInternalSelection(dataTransfer: DataTransfer): SourceSelection | null {
  const raw =
    dataTransfer.getData(INTERNAL_TRANSFER_MIME) || dataTransfer.getData("text/plain");
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as {
      kind?: string;
      fromSourceId?: string;
      paths?: unknown;
    };
    if (parsed.kind !== "infimount-transfer" || typeof parsed.fromSourceId !== "string") {
      return null;
    }
    if (!Array.isArray(parsed.paths)) return null;
    const paths = parsed.paths.filter((path): path is string => typeof path === "string");
    if (paths.length === 0) return null;
    return { sourceId: parsed.fromSourceId, paths };
  } catch {
    return null;
  }
}

function isTextPreview(path: string) {
  return /\.(?:txt|md|markdown|json|jsonl|csv|tsv|ya?ml|toml|xml|html?|css|js|jsx|ts|tsx|py|rs|go|java|sh|sql|log)$/i.test(
    path,
  );
}

export function AgentTaskHub() {
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<HubMode>("tasks");
  const [storages, setStorages] = useState<TaskStorage[]>([]);
  const [workspaces, setWorkspaces] = useState<AgentWorkspace[]>([]);
  const [sourceSelection, setSourceSelection] = useState<SourceSelection | null>(null);
  const [manualSourceId, setManualSourceId] = useState("");
  const [manualPaths, setManualPaths] = useState("");
  const [workspaceId, setWorkspaceId] = useState("");
  const [title, setTitle] = useState("");
  const [objective, setObjective] = useState("");
  const [requestedOutputsText, setRequestedOutputsText] = useState("summary.md");
  const [preflight, setPreflight] = useState<AgentTaskPreflightOutput | null>(null);
  const [prepared, setPrepared] = useState<PrepareAgentTaskOutput | null>(null);
  const [busy, setBusy] = useState(false);
  const [isDropTarget, setIsDropTarget] = useState(false);

  const [taskWorkspaceId, setTaskWorkspaceId] = useState("");
  const [tasks, setTasks] = useState<AgentTaskSummary[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState("");
  const [outputs, setOutputs] = useState<AgentTaskOutputEntry[]>([]);
  const [selectedOutputs, setSelectedOutputs] = useState<Set<string>>(new Set());
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [previewText, setPreviewText] = useState("");
  const [destinationStorageId, setDestinationStorageId] = useState("");
  const [destinationDir, setDestinationDir] = useState("");
  const [publishResult, setPublishResult] = useState<PublishAgentTaskResult | null>(null);

  const loadEnvironment = useCallback(async () => {
    const [rawStorages, nextWorkspaces] = await Promise.all([
      listStorages(),
      listAgentWorkspaces(),
    ]);
    const nextStorages = (rawStorages as unknown[])
      .map(mapTaskStorage)
      .filter((storage): storage is TaskStorage => storage !== null);
    setStorages(nextStorages);
    setWorkspaces(nextWorkspaces);
    setManualSourceId((current) => current || nextStorages.find((item) => item.enabled)?.id || "");
    setDestinationStorageId(
      (current) => current || nextStorages.find((item) => item.enabled && !item.readOnly)?.id || "",
    );
    const compatible = nextWorkspaces.find((workspace) => {
      const storage = nextStorages.find((item) => item.id === workspace.storageId);
      return workspace.accessProfile === "read_write" && isLocalStorage(storage) && storage?.enabled;
    });
    setWorkspaceId((current) => current || compatible?.id || "");
    setTaskWorkspaceId((current) => current || compatible?.id || nextWorkspaces[0]?.id || "");
  }, []);

  useEffect(() => {
    if (!open) return;
    void loadEnvironment().catch((error) => {
      toast({
        title: "Agent Tasks unavailable",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    });
  }, [loadEnvironment, open]);

  useEffect(() => {
    const handleDragStart = (event: DragEvent) => {
      if (!event.dataTransfer) return;
      const selection = parseInternalSelection(event.dataTransfer);
      if (selection) setSourceSelection(selection);
    };
    window.addEventListener("dragstart", handleDragStart, true);
    return () => window.removeEventListener("dragstart", handleDragStart, true);
  }, []);

  const storageById = useMemo(
    () => new Map(storages.map((storage) => [storage.id, storage])),
    [storages],
  );
  const compatibleWorkspaces = useMemo(
    () =>
      workspaces.filter((workspace) => {
        const storage = storageById.get(workspace.storageId);
        return workspace.accessProfile === "read_write" && isLocalStorage(storage) && storage?.enabled;
      }),
    [storageById, workspaces],
  );
  const writableStorages = useMemo(
    () => storages.filter((storage) => storage.enabled && !storage.readOnly),
    [storages],
  );
  const localWritableStorages = useMemo(
    () => writableStorages.filter((storage) => isLocalStorage(storage)),
    [writableStorages],
  );

  const effectiveSelection = useMemo<SourceSelection | null>(() => {
    if (sourceSelection) return sourceSelection;
    const paths = manualPaths
      .split(/\r?\n/)
      .map((path) => path.trim())
      .filter(Boolean);
    return manualSourceId && paths.length > 0 ? { sourceId: manualSourceId, paths } : null;
  }, [manualPaths, manualSourceId, sourceSelection]);

  useEffect(() => {
    setPreflight(null);
    setPrepared(null);
    if (!effectiveSelection) return;
    if (!title.trim()) {
      const first = effectiveSelection.paths[0];
      setTitle(
        effectiveSelection.paths.length === 1
          ? `Work with ${basename(first)}`
          : `Work with ${effectiveSelection.paths.length} selected items`,
      );
    }
  }, [effectiveSelection, title]);

  const selectedTaskWorkspace = workspaces.find((workspace) => workspace.id === taskWorkspaceId) ?? null;
  const selectedTask = tasks.find((task) => task.taskId === selectedTaskId) ?? null;

  const reloadTasks = useCallback(async (targetWorkspaceId: string) => {
    if (!targetWorkspaceId) {
      setTasks([]);
      setSelectedTaskId("");
      return;
    }
    const next = await listAgentTasks(targetWorkspaceId);
    setTasks(next);
    setSelectedTaskId((current) =>
      current && next.some((task) => task.taskId === current) ? current : next[0]?.taskId ?? "",
    );
  }, []);

  useEffect(() => {
    if (!open || mode !== "tasks" || !taskWorkspaceId) return;
    void reloadTasks(taskWorkspaceId).catch((error) => {
      toast({
        title: "Tasks could not be loaded",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    });
  }, [mode, open, reloadTasks, taskWorkspaceId]);

  const reloadOutputs = useCallback(async () => {
    if (!taskWorkspaceId || !selectedTaskId) {
      setOutputs([]);
      setSelectedOutputs(new Set());
      return;
    }
    const result = await listAgentTaskOutputs(taskWorkspaceId, selectedTaskId);
    setOutputs(result.outputs);
    setSelectedOutputs(new Set(result.outputs.map((output) => output.taskPath)));
    setPreviewPath(null);
    setPreviewText("");
    setPublishResult(null);
  }, [selectedTaskId, taskWorkspaceId]);

  useEffect(() => {
    if (!open || mode !== "tasks" || !selectedTaskId) return;
    void reloadOutputs().catch((error) => {
      toast({
        title: "Outputs could not be loaded",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    });
  }, [mode, open, reloadOutputs, selectedTaskId]);

  const createDedicatedWorkspace = async () => {
    const localStorage = localWritableStorages[0];
    if (!localStorage) {
      toast({
        title: "Local storage required",
        description: "Add a writable Local Filesystem storage before creating an Agent Task workspace.",
        variant: "destructive",
      });
      return;
    }
    setBusy(true);
    try {
      const existingNames = new Set(workspaces.map((workspace) => workspace.name.toLowerCase()));
      let suffix = 1;
      let name = "Agent tasks";
      while (existingNames.has(name.toLowerCase())) {
        suffix += 1;
        name = `Agent tasks ${suffix}`;
      }
      const workspace = await createAgentWorkspace({
        storageId: localStorage.id,
        name,
        rootPath: defaultWorkspacePath(name),
        templateId: "coding",
        accessProfile: "read_write",
        applyPolicy: true,
      });
      await loadEnvironment();
      setWorkspaceId(workspace.id);
      setTaskWorkspaceId(workspace.id);
      toast({
        title: "Task workspace created",
        description: "The workspace has read/write MCP access scoped to its own root.",
      });
    } catch (error) {
      toast({
        title: "Task workspace could not be created",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setBusy(false);
    }
  };

  const buildPreparationRequest = () => {
    if (!effectiveSelection || !workspaceId) return null;
    return {
      sourceStorageId: effectiveSelection.sourceId,
      sourcePaths: effectiveSelection.paths,
      workspaceId,
      title: title.trim(),
      objective: objective.trim(),
      requestedOutputs: normalizeRequestedOutputs(requestedOutputsText),
    };
  };

  const runPreflight = async () => {
    const request = buildPreparationRequest();
    if (!request || !request.title || !request.objective) return;
    setBusy(true);
    setPrepared(null);
    try {
      setPreflight(await preflightAgentTask(request));
    } catch (error) {
      setPreflight(null);
      toast({
        title: "Task preflight failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setBusy(false);
    }
  };

  const runPrepare = async () => {
    const request = buildPreparationRequest();
    if (!request || !preflight) return;
    setBusy(true);
    try {
      const result = await prepareAgentTask(request);
      setPrepared(result);
      setTaskWorkspaceId(result.workspaceId);
      await reloadTasks(result.workspaceId);
      toast({
        title: "Agent Task prepared",
        description: `${result.preparedFiles} files are ready in ${result.taskRoot}.`,
      });
    } catch (error) {
      toast({
        title: "Task preparation failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setBusy(false);
    }
  };

  const copyHandoff = async () => {
    if (!prepared) return;
    const workspace = workspaces.find((item) => item.id === prepared.workspaceId);
    const storage = workspace ? storageById.get(workspace.storageId) : null;
    if (!workspace || !storage) return;
    const mcpPath = `/${storage.name}/${joinPath(workspace.rootPath, prepared.taskRoot)}`;
    const text = `Use the Infimount Agent Task at ${mcpPath}. Read TASK.md, work only inside that task workspace, and put deliverables under outputs/. Do not publish or overwrite remote storage; I will review outputs in Infimount.`;
    try {
      await navigator.clipboard.writeText(text);
      toast({ title: "Agent instruction copied" });
    } catch {
      toast({ title: "Could not copy instruction", variant: "destructive" });
    }
  };

  const previewOutput = async (output: AgentTaskOutputEntry) => {
    if (!selectedTaskWorkspace || !selectedTask || !isTextPreview(output.taskPath)) {
      setPreviewPath(output.taskPath);
      setPreviewText("Preview is available for text, Markdown, JSON, CSV, logs, and common source files.");
      return;
    }
    setPreviewPath(output.taskPath);
    setPreviewText("Loading preview…");
    try {
      const path = joinPath(selectedTaskWorkspace.rootPath, selectedTask.taskRoot, output.taskPath);
      const result = await readFileRange(
        selectedTaskWorkspace.storageId,
        path,
        0,
        MAX_TEXT_PREVIEW_BYTES,
      );
      const text = new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(result.bytes));
      setPreviewText(result.truncated ? `${text}\n\n[Preview truncated]` : text);
    } catch (error) {
      setPreviewText(error instanceof Error ? error.message : String(error));
    }
  };

  const runPublish = async () => {
    if (!selectedTask || !taskWorkspaceId || selectedOutputs.size === 0 || !destinationStorageId) {
      return;
    }
    setBusy(true);
    setPublishResult(null);
    try {
      const result = await publishAgentTaskOutputs({
        workspaceId: taskWorkspaceId,
        taskId: selectedTask.taskId,
        outputs: outputs
          .filter((output) => selectedOutputs.has(output.taskPath))
          .map((output) => ({ taskPath: output.taskPath, sha256: output.sha256 })),
        destinationStorageId,
        destinationDir,
      });
      setPublishResult(result);
      const clean = result.failed === 0 && result.stale === 0 && result.conflicts === 0;
      toast({
        title: clean ? "Outputs published" : "Publication completed with exceptions",
        description: `${result.published} published · ${result.conflicts} conflicts · ${result.stale} changed · ${result.failed} failed`,
        variant: clean ? "default" : "destructive",
      });
    } catch (error) {
      toast({
        title: "Publication failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setBusy(false);
    }
  };

  const handleDrop = (event: React.DragEvent) => {
    event.preventDefault();
    event.stopPropagation();
    setIsDropTarget(false);
    const selection = parseInternalSelection(event.dataTransfer);
    if (!selection) return;
    setSourceSelection(selection);
    setManualSourceId(selection.sourceId);
    setManualPaths(selection.paths.join("\n"));
    setPreflight(null);
    setPrepared(null);
    setMode("prepare");
    setOpen(true);
  };

  return (
    <>
      <div
        className="fixed bottom-5 right-5 z-40"
        onDragOver={(event) => {
          if (!parseInternalSelection(event.dataTransfer)) return;
          event.preventDefault();
          setIsDropTarget(true);
        }}
        onDragLeave={() => setIsDropTarget(false)}
        onDrop={handleDrop}
      >
        <Button
          className={cn(
            "gap-2 rounded-full shadow-lg transition-transform",
            isDropTarget && "scale-105 ring-2 ring-primary/40",
          )}
          onClick={() => {
            setMode(sourceSelection ? "prepare" : "tasks");
            setOpen(true);
          }}
          title="Agent Tasks — click to review tasks or drag selected files here"
        >
          <Sparkles className="h-4 w-4" />
          Agent Tasks
        </Button>
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="flex max-h-[88vh] max-w-6xl flex-col overflow-hidden p-0">
          <DialogHeader className="border-b px-5 py-4">
            <div className="flex items-start justify-between gap-4 pr-8">
              <div className="flex items-start gap-3">
                <div className="mt-0.5 rounded-lg border bg-muted/60 p-2">
                  <Bot className="h-4 w-4" />
                </div>
                <div>
                  <DialogTitle>Agent Tasks</DialogTitle>
                  <DialogDescription>
                    Bring selected files to an agent, review what comes back, and publish only approved bytes.
                  </DialogDescription>
                </div>
              </div>
              <div className="flex gap-1 rounded-lg bg-muted p-1">
                <Button
                  size="sm"
                  variant={mode === "prepare" ? "secondary" : "ghost"}
                  onClick={() => setMode("prepare")}
                >
                  Prepare
                </Button>
                <Button
                  size="sm"
                  variant={mode === "tasks" ? "secondary" : "ghost"}
                  onClick={() => setMode("tasks")}
                >
                  Review
                </Button>
              </div>
            </div>
          </DialogHeader>

          <ScrollArea className="min-h-0 flex-1">
            {mode === "prepare" ? (
              <div className="space-y-5 p-5">
                <section className="rounded-xl border bg-background p-4">
                  <div className="mb-3 flex items-start justify-between gap-3">
                    <div>
                      <h3 className="text-sm font-medium">1. Selected inputs</h3>
                      <p className="text-xs text-muted-foreground">
                        Drag selected items from the file browser onto the Agent Tasks button, or enter storage-relative paths below.
                      </p>
                    </div>
                    {sourceSelection && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setSourceSelection(null)}
                      >
                        Edit manually
                      </Button>
                    )}
                  </div>
                  <div className="grid gap-3 md:grid-cols-[240px_1fr]">
                    <div className="space-y-2">
                      <Label>Source storage</Label>
                      <Select
                        value={sourceSelection?.sourceId ?? manualSourceId}
                        onValueChange={(value) => {
                          setSourceSelection(null);
                          setManualSourceId(value);
                          setPreflight(null);
                        }}
                      >
                        <SelectTrigger><SelectValue placeholder="Choose source" /></SelectTrigger>
                        <SelectContent>
                          {storages.filter((storage) => storage.enabled).map((storage) => (
                            <SelectItem key={storage.id} value={storage.id}>{storage.name}</SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="space-y-2">
                      <Label>Paths</Label>
                      <Textarea
                        value={sourceSelection ? sourceSelection.paths.join("\n") : manualPaths}
                        onChange={(event) => {
                          setSourceSelection(null);
                          setManualPaths(event.target.value);
                          setPreflight(null);
                        }}
                        className="min-h-24 font-mono text-xs"
                        placeholder={"reports/customer.csv\nlogs/incident/"}
                      />
                    </div>
                  </div>
                </section>

                <section className="rounded-xl border bg-background p-4">
                  <h3 className="text-sm font-medium">2. Task</h3>
                  <div className="mt-3 grid gap-3 md:grid-cols-2">
                    <div className="space-y-2 md:col-span-2">
                      <Label>Title</Label>
                      <Input value={title} onChange={(event) => { setTitle(event.target.value); setPreflight(null); }} />
                    </div>
                    <div className="space-y-2 md:col-span-2">
                      <Label>What should the agent do?</Label>
                      <Textarea
                        value={objective}
                        onChange={(event) => { setObjective(event.target.value); setPreflight(null); }}
                        className="min-h-28"
                        placeholder="Analyze these files and summarize the important findings."
                      />
                    </div>
                    <div className="space-y-2 md:col-span-2">
                      <Label>Requested outputs</Label>
                      <Textarea
                        value={requestedOutputsText}
                        onChange={(event) => { setRequestedOutputsText(event.target.value); setPreflight(null); }}
                        className="min-h-20 font-mono text-xs"
                        placeholder={"summary.md\ninvalid-records.csv"}
                      />
                      <p className="text-xs text-muted-foreground">
                        One name per line. Infimount places them under <code>outputs/</code>.
                      </p>
                    </div>
                  </div>
                </section>

                <section className="rounded-xl border bg-background p-4">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h3 className="text-sm font-medium">3. Controlled workspace</h3>
                      <p className="text-xs text-muted-foreground">
                        Agent Tasks use a local read/write Agent Workspace; the original source is copied, not exposed by the task.
                      </p>
                    </div>
                    {compatibleWorkspaces.length === 0 && (
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => void createDedicatedWorkspace()}
                        disabled={busy}
                      >
                        <FolderPlus className="mr-2 h-4 w-4" /> Create task workspace
                      </Button>
                    )}
                  </div>
                  <div className="mt-3 space-y-2">
                    <Label>Workspace</Label>
                    <Select value={workspaceId} onValueChange={(value) => { setWorkspaceId(value); setPreflight(null); }}>
                      <SelectTrigger><SelectValue placeholder="Choose local read/write workspace" /></SelectTrigger>
                      <SelectContent>
                        {compatibleWorkspaces.map((workspace) => (
                          <SelectItem key={workspace.id} value={workspace.id}>
                            {workspace.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {compatibleWorkspaces.length === 0 && (
                      <p className="text-xs text-muted-foreground">
                        {localWritableStorages.length > 0
                          ? "No local read/write Agent Workspace exists yet. Create one explicitly above."
                          : "Add a writable Local Filesystem storage first; Agent Tasks do not stage directly in cloud storage."}
                      </p>
                    )}
                  </div>
                </section>

                {preflight && (
                  <section className="rounded-xl border bg-muted/20 p-4">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="secondary">{preflight.fileCount} files</Badge>
                      <Badge variant="secondary">{preflight.directoryCount} folders</Badge>
                      <Badge variant="secondary">{formatTaskBytes(preflight.totalBytes)}</Badge>
                      <Badge variant="outline" className="gap-1"><ShieldCheck className="h-3 w-3" /> source copied</Badge>
                    </div>
                    {preflight.warnings.length > 0 && (
                      <div className="mt-3 space-y-2 text-xs text-amber-700 dark:text-amber-300">
                        {preflight.warnings.map((warning) => <p key={warning}>{warning}</p>)}
                      </div>
                    )}
                  </section>
                )}

                {prepared && (
                  <section className="rounded-xl border border-primary/25 bg-primary/5 p-4">
                    <div className="flex items-start gap-3">
                      <Check className="mt-0.5 h-5 w-5 text-primary" />
                      <div className="min-w-0 flex-1">
                        <h3 className="text-sm font-medium">Task ready</h3>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {prepared.preparedFiles} files · {formatTaskBytes(prepared.totalBytes)} · <span className="font-mono">{prepared.taskRoot}</span>
                        </p>
                        <div className="mt-3 flex flex-wrap gap-2">
                          <Button size="sm" variant="outline" onClick={() => void copyHandoff()}>
                            <Clipboard className="mr-2 h-4 w-4" /> Copy agent instruction
                          </Button>
                          <Button size="sm" onClick={() => { setSelectedTaskId(prepared.taskId); setMode("tasks"); }}>
                            Review outputs
                          </Button>
                        </div>
                      </div>
                    </div>
                  </section>
                )}

                <div className="flex justify-end gap-2">
                  <Button
                    variant="outline"
                    onClick={() => void runPreflight()}
                    disabled={busy || !effectiveSelection || !workspaceId || !title.trim() || !objective.trim()}
                  >
                    {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <ShieldCheck className="mr-2 h-4 w-4" />}
                    Preflight
                  </Button>
                  <Button onClick={() => void runPrepare()} disabled={busy || !preflight || !!prepared}>
                    <Send className="mr-2 h-4 w-4" /> Prepare task
                  </Button>
                </div>
              </div>
            ) : (
              <div className="grid min-h-[620px] grid-cols-[280px_1fr]">
                <aside className="border-r bg-muted/20 p-3">
                  <div className="space-y-2">
                    <Label>Workspace</Label>
                    <Select value={taskWorkspaceId} onValueChange={setTaskWorkspaceId}>
                      <SelectTrigger><SelectValue placeholder="Choose workspace" /></SelectTrigger>
                      <SelectContent>
                        {compatibleWorkspaces.map((workspace) => (
                          <SelectItem key={workspace.id} value={workspace.id}>{workspace.name}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="mt-4 flex items-center justify-between">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Tasks</span>
                    <Button size="icon" variant="ghost" className="h-7 w-7" onClick={() => void reloadTasks(taskWorkspaceId)}>
                      <RefreshCw className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                  <div className="mt-2 space-y-1">
                    {tasks.length === 0 ? (
                      <p className="px-2 py-4 text-xs text-muted-foreground">No prepared tasks in this workspace.</p>
                    ) : tasks.map((task) => (
                      <button
                        key={task.taskId}
                        type="button"
                        className={cn(
                          "w-full rounded-lg p-2 text-left text-sm outline-none transition-colors",
                          selectedTaskId === task.taskId ? "bg-background shadow-sm" : "text-muted-foreground hover:bg-background/70",
                        )}
                        onClick={() => setSelectedTaskId(task.taskId)}
                      >
                        <div className="truncate font-medium text-foreground">{task.title}</div>
                        <div className="mt-1 text-[11px] text-muted-foreground">
                          {task.preparedFiles} inputs · {formatTaskBytes(task.preparedBytes)}
                        </div>
                      </button>
                    ))}
                  </div>
                </aside>

                <div className="min-w-0 space-y-4 p-5">
                  {!selectedTask ? (
                    <div className="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">
                      Select a task to review its outputs.
                    </div>
                  ) : (
                    <>
                      <section className="rounded-xl border bg-background p-4">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <h3 className="text-sm font-medium">{selectedTask.title}</h3>
                            <p className="mt-1 font-mono text-xs text-muted-foreground">{selectedTask.taskRoot}</p>
                          </div>
                          <Button size="sm" variant="outline" onClick={() => void reloadOutputs()}>
                            <RefreshCw className="mr-2 h-4 w-4" /> Refresh outputs
                          </Button>
                        </div>
                      </section>

                      <div className="grid gap-4 lg:grid-cols-[1fr_1fr]">
                        <section className="rounded-xl border bg-background p-4">
                          <div className="mb-3 flex items-center justify-between">
                            <h3 className="text-sm font-medium">Outputs</h3>
                            <Badge variant="outline">{outputs.length}</Badge>
                          </div>
                          <div className="max-h-72 space-y-1 overflow-auto">
                            {outputs.length === 0 ? (
                              <p className="py-6 text-center text-xs text-muted-foreground">
                                No files under outputs/ yet. Let the agent finish, then refresh.
                              </p>
                            ) : outputs.map((output) => (
                              <div key={output.taskPath} className="flex items-center gap-2 rounded-lg px-2 py-2 hover:bg-muted/50">
                                <Checkbox
                                  checked={selectedOutputs.has(output.taskPath)}
                                  onCheckedChange={(checked) => {
                                    setSelectedOutputs((current) => {
                                      const next = new Set(current);
                                      if (checked === true) next.add(output.taskPath);
                                      else next.delete(output.taskPath);
                                      return next;
                                    });
                                  }}
                                  aria-label={`Select ${output.taskPath}`}
                                />
                                <button type="button" className="min-w-0 flex-1 text-left" onClick={() => void previewOutput(output)}>
                                  <div className="truncate font-mono text-xs">{output.taskPath}</div>
                                  <div className="text-[11px] text-muted-foreground">{formatTaskBytes(output.byteSize)} · {output.sha256.slice(0, 12)}…</div>
                                </button>
                              </div>
                            ))}
                          </div>
                        </section>

                        <section className="rounded-xl border bg-background p-4">
                          <h3 className="text-sm font-medium">Preview</h3>
                          <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">{previewPath ?? "Choose an output"}</p>
                          <pre className="mt-3 max-h-64 min-h-44 overflow-auto whitespace-pre-wrap rounded-lg bg-muted/50 p-3 font-mono text-xs">
                            {previewPath ? previewText : "Select an output file to inspect it before publishing."}
                          </pre>
                        </section>
                      </div>

                      <section className="rounded-xl border bg-background p-4">
                        <div className="mb-3 flex items-start gap-3">
                          <FileCheck2 className="mt-0.5 h-4 w-4" />
                          <div>
                            <h3 className="text-sm font-medium">Publish approved outputs</h3>
                            <p className="text-xs text-muted-foreground">
                              New files only. Infimount snapshots the reviewed bytes, rechecks SHA-256, and uses atomic create-if-absent on the destination.
                            </p>
                          </div>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[260px_1fr]">
                          <div className="space-y-2">
                            <Label>Destination storage</Label>
                            <Select value={destinationStorageId} onValueChange={setDestinationStorageId}>
                              <SelectTrigger><SelectValue placeholder="Choose destination" /></SelectTrigger>
                              <SelectContent>
                                {writableStorages.map((storage) => (
                                  <SelectItem key={storage.id} value={storage.id}>{storage.name}</SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          </div>
                          <div className="space-y-2">
                            <Label>Destination folder</Label>
                            <Input
                              className="font-mono"
                              value={destinationDir}
                              onChange={(event) => setDestinationDir(event.target.value)}
                              placeholder="reports/agent-output"
                            />
                          </div>
                        </div>
                        <div className="mt-4 flex justify-end">
                          <Button onClick={() => void runPublish()} disabled={busy || selectedOutputs.size === 0 || !destinationStorageId}>
                            {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <FileCheck2 className="mr-2 h-4 w-4" />}
                            Publish {selectedOutputs.size || ""} approved
                          </Button>
                        </div>

                        {publishResult && (
                          <div className="mt-4 rounded-lg bg-muted/40 p-3">
                            <div className="flex flex-wrap gap-2 text-xs">
                              <Badge variant="secondary">{publishResult.published} published</Badge>
                              {publishResult.conflicts > 0 && <Badge variant="outline">{publishResult.conflicts} conflicts</Badge>}
                              {publishResult.stale > 0 && <Badge variant="outline">{publishResult.stale} changed</Badge>}
                              {publishResult.failed > 0 && <Badge variant="outline">{publishResult.failed} failed</Badge>}
                            </div>
                            <div className="mt-3 space-y-1">
                              {publishResult.results.map((item) => (
                                <div key={`${item.taskPath}:${item.destinationPath}`} className="flex items-center justify-between gap-3 text-xs">
                                  <span className="truncate font-mono">{item.taskPath}</span>
                                  <Badge variant={item.status === "published" ? "secondary" : "outline"}>{item.status}</Badge>
                                </div>
                              ))}
                            </div>
                            {!publishResult.receiptWritten && (
                              <p className="mt-2 text-xs text-muted-foreground">Publication finished, but the local task receipt could not be updated.</p>
                            )}
                          </div>
                        )}
                      </section>
                    </>
                  )}
                </div>
              </div>
            )}
          </ScrollArea>
        </DialogContent>
      </Dialog>
    </>
  );
}
