import { useEffect, useMemo, useState } from "react";
import {
  Bot,
  CheckCircle2,
  Clock3,
  FolderLock,
  NotebookPen,
  RefreshCw,
  RotateCcw,
  Save,
  Trash2,
} from "lucide-react";

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
import { Badge } from "@/components/ui/badge";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import {
  AGENT_WORKSPACE_TEMPLATES,
  appendWorkspaceMemory,
  createAgentWorkspace,
  createWorkspaceCheckpoint,
  defaultWorkspacePath,
  deleteAgentWorkspace,
  deleteAgentWorkspaceWithFiles,
  listAgentWorkspaceCheckpoints,
  listAgentWorkspaces,
  listWorkspaceMemoryFiles,
  readWorkspaceMemoryFile,
  restoreWorkspaceMemoryCheckpoint,
  type AgentWorkspace,
  type AgentWorkspaceCheckpoint,
  type AgentWorkspaceTemplateId,
} from "@/lib/agentWorkspaces";
import { listActivityLogEvents, type ActivityLogEvent } from "@/lib/activityLog";
import { cn } from "@/lib/utils";
import type { McpAuditEvent, StorageConfig } from "@/types/storage";

interface AgentWorkspacesDialogProps {
  open: boolean;
  storages: StorageConfig[];
  auditEvents?: McpAuditEvent[];
  onOpenChange: (open: boolean) => void;
  onSelectStorage: (storageId: string) => void;
}

export function AgentWorkspacesDialog({
  open,
  storages,
  auditEvents = [],
  onOpenChange,
  onSelectStorage,
}: AgentWorkspacesDialogProps) {
  const [workspaces, setWorkspaces] = useState<AgentWorkspace[]>([]);
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState<string | null>(null);
  const [storageId, setStorageId] = useState("");
  const [name, setName] = useState("Coding workspace");
  const [rootPath, setRootPath] = useState(defaultWorkspacePath("Coding workspace"));
  const [templateId, setTemplateId] = useState<AgentWorkspaceTemplateId>("coding");
  const [applyPolicy, setApplyPolicy] = useState(true);
  const [isCreating, setIsCreating] = useState(false);
  const [memoryFiles, setMemoryFiles] = useState<string[]>([]);
  const [selectedMemoryFile, setSelectedMemoryFile] = useState<string | null>(null);
  const [memoryContent, setMemoryContent] = useState("");
  const [memoryAppendText, setMemoryAppendText] = useState("");
  const [checkpoints, setCheckpoints] = useState<AgentWorkspaceCheckpoint[]>([]);
  const [selectedCheckpointId, setSelectedCheckpointId] = useState<string>("");
  const [isMemoryBusy, setIsMemoryBusy] = useState(false);
  const [isCheckpointBusy, setIsCheckpointBusy] = useState(false);
  const [restoreConfirmationOpen, setRestoreConfirmationOpen] = useState(false);
  const [activityEvents, setActivityEvents] = useState<ActivityLogEvent[]>([]);
  const [deleteMode, setDeleteMode] = useState<"registration" | "files" | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const selectedWorkspace = useMemo(
    () => workspaces.find((workspace) => workspace.id === selectedWorkspaceId) ?? null,
    [selectedWorkspaceId, workspaces],
  );
  const selectedStorage = storages.find((storage) => storage.id === storageId) ?? storages[0];
  const selectedWorkspaceStorage = selectedWorkspace
    ? storages.find((storage) => storage.id === selectedWorkspace.storageId)
    : null;
  const workspaceAuditItems = useMemo(
    () => selectedWorkspace ? buildWorkspaceAuditItems(selectedWorkspace, activityEvents, auditEvents) : [],
    [activityEvents, auditEvents, selectedWorkspace],
  );

  const refreshWorkspaceActivity = () => setActivityEvents(listActivityLogEvents());

  useEffect(() => {
    if (!open) return;
    let active = true;
    void listAgentWorkspaces().then((next) => {
      if (!active) return;
      setWorkspaces(next);
      setSelectedWorkspaceId((current) => current ?? next[0]?.id ?? null);
      setStorageId((current) => current || storages[0]?.id || "");
      refreshWorkspaceActivity();
    });
    return () => { active = false; };
  }, [open, storages]);

  useEffect(() => {
    setRootPath(defaultWorkspacePath(name));
  }, [name]);

  useEffect(() => {
    if (!selectedWorkspace || !open) {
      setMemoryFiles([]);
      setSelectedMemoryFile(null);
      setMemoryContent("");
      setCheckpoints([]);
      setSelectedCheckpointId("");
      return;
    }

    let active = true;
    setMemoryFiles(selectedWorkspace.memoryFiles);
    setSelectedMemoryFile((current) => current ?? selectedWorkspace.memoryFiles[0] ?? null);
    void listAgentWorkspaceCheckpoints(selectedWorkspace)
      .then((items) => {
        if (active) setCheckpoints(items);
      })
      .catch(() => {
        if (active) setCheckpoints([]);
      });
    void listWorkspaceMemoryFiles(selectedWorkspace)
      .then((files) => {
        if (!active) return;
        setMemoryFiles(files);
        setSelectedMemoryFile((current) => current ?? files[0] ?? null);
      })
      .catch(() => undefined);

    return () => {
      active = false;
    };
  }, [open, selectedWorkspace]);

  useEffect(() => {
    if (!selectedWorkspace || !selectedMemoryFile || !open) return;
    let active = true;
    setIsMemoryBusy(true);
    void readWorkspaceMemoryFile(selectedWorkspace, selectedMemoryFile)
      .then((content) => {
        if (!active) return;
        setMemoryContent(content);
      })
      .catch((error) => {
        if (!active) return;
        setMemoryContent("");
        toast({
          title: "Memory file could not be read",
          description: error instanceof Error ? error.message : String(error),
          variant: "destructive",
        });
      })
      .finally(() => {
        if (active) setIsMemoryBusy(false);
      });
    return () => {
      active = false;
    };
  }, [open, selectedMemoryFile, selectedWorkspace]);

  const handleCreate = async () => {
    if (!selectedStorage) return;
    setIsCreating(true);
    try {
      const workspace = await createAgentWorkspace({
        storageId: selectedStorage.id,
        name,
        rootPath,
        templateId,
        accessProfile: "read_only",
        applyPolicy,
      });
      const next = await listAgentWorkspaces();
      setWorkspaces(next);
      setSelectedWorkspaceId(workspace.id);
      refreshWorkspaceActivity();
      toast({
        title: "Workspace created",
        description: applyPolicy
          ? "MCP access is now scoped to this workspace root."
          : "Workspace files were created without changing MCP policy.",
      });
    } catch (error) {
      toast({
        title: "Workspace could not be created",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsCreating(false);
    }
  };

  const handleAppendMemory = async () => {
    if (!selectedWorkspace || !selectedMemoryFile || !memoryAppendText.trim()) return;
    setIsMemoryBusy(true);
    try {
      const next = await appendWorkspaceMemory(
        selectedWorkspace,
        selectedMemoryFile,
        memoryAppendText,
      );
      setMemoryContent(next);
      setMemoryAppendText("");
      refreshWorkspaceActivity();
      toast({ title: "Memory appended", description: `${selectedMemoryFile} was updated.` });
    } catch (error) {
      toast({
        title: "Memory could not be updated",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsMemoryBusy(false);
    }
  };

  const handleCheckpoint = async () => {
    if (!selectedWorkspace) return;
    setIsCheckpointBusy(true);
    try {
      const checkpoint = await createWorkspaceCheckpoint(selectedWorkspace);
      const nextCheckpoints = await listAgentWorkspaceCheckpoints(selectedWorkspace);
      setCheckpoints(nextCheckpoints);
      setSelectedCheckpointId(checkpoint.id);
      setWorkspaces(await listAgentWorkspaces());
      refreshWorkspaceActivity();
      toast({
        title: "Checkpoint saved",
        description: "Memory files were captured in the workspace checkpoint manifest.",
      });
    } catch (error) {
      toast({
        title: "Checkpoint failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsCheckpointBusy(false);
    }
  };

  const handleRestore = async () => {
    if (!selectedWorkspace || !selectedCheckpointId) return;
    setIsCheckpointBusy(true);
    try {
      await restoreWorkspaceMemoryCheckpoint(selectedWorkspace, selectedCheckpointId, true);
      if (selectedMemoryFile) {
        setMemoryContent(await readWorkspaceMemoryFile(selectedWorkspace, selectedMemoryFile));
      }
      refreshWorkspaceActivity();
      setRestoreConfirmationOpen(false);
      toast({ title: "Checkpoint restored", description: "Memory files were restored." });
    } catch (error) {
      toast({
        title: "Restore failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsCheckpointBusy(false);
    }
  };

  const handleDeleteWorkspace = async () => {
    if (!selectedWorkspace || !deleteMode) return;
    setIsDeleting(true);
    try {
      if (deleteMode === "files") {
        await deleteAgentWorkspaceWithFiles(selectedWorkspace.id);
      } else {
        await deleteAgentWorkspace(selectedWorkspace.id);
      }
      const next = await listAgentWorkspaces();
      setWorkspaces(next);
      setSelectedWorkspaceId(next[0]?.id ?? null);
      refreshWorkspaceActivity();
      toast({
        title: deleteMode === "files" ? "Workspace and files deleted" : "Workspace registration removed",
        description: deleteMode === "files"
          ? "The scoped policy, registration, and workspace root were removed."
          : "The scoped policy and registration were removed. Storage files were preserved.",
      });
      setDeleteMode(null);
    } catch (error) {
      toast({
        title: "Workspace deletion failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setIsDeleting(false);
    }
  };

  const canCreate = Boolean(selectedStorage && name.trim() && rootPath.trim());

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[86vh] max-w-5xl flex-col overflow-hidden p-0">
        <DialogHeader className="border-b px-5 py-4">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 rounded-lg border bg-muted/60 p-2">
              <Bot className="h-4 w-4" />
            </div>
            <div>
              <DialogTitle>Agent Workspaces</DialogTitle>
              <DialogDescription>
                Create scoped folders for agents, apply MCP policy, and keep visible memory files.
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="grid min-h-0 flex-1 grid-cols-[260px_1fr]">
          <aside className="min-h-0 border-r bg-muted/20">
            <div className="border-b px-4 py-3">
              <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Workspaces
              </div>
            </div>
            <ScrollArea className="h-[58vh]">
              <div className="space-y-1 p-2">
                {workspaces.length === 0 ? (
                  <div className="px-3 py-6 text-sm text-muted-foreground">
                    No workspaces yet. Create one on any storage.
                  </div>
                ) : (
                  workspaces.map((workspace) => (
                    <button
                      key={workspace.id}
                      type="button"
                      className={cn(
                        "w-full rounded-lg px-3 py-2 text-left text-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/30",
                        selectedWorkspaceId === workspace.id
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
                      )}
                      onClick={() => setSelectedWorkspaceId(workspace.id)}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate font-medium">{workspace.name}</span>
                        <Badge variant="outline" className="shrink-0 text-[10px]">
                          {workspace.templateId}
                        </Badge>
                      </div>
                      <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
                        {workspace.rootPath}
                      </div>
                    </button>
                  ))
                )}
              </div>
            </ScrollArea>
          </aside>

          <ScrollArea className="min-h-0">
            <div className="space-y-5 p-5">
              <section className="rounded-xl border bg-background p-4">
                <div className="mb-4 flex items-center justify-between gap-3">
                  <div>
                    <h3 className="text-sm font-medium">Create workspace</h3>
                    <p className="text-xs text-muted-foreground">
                      Files are written through OpenDAL. Credentials stay local.
                    </p>
                  </div>
                  {applyPolicy && (
                    <Badge variant="secondary" className="gap-1">
                      <FolderLock className="h-3 w-3" /> MCP scoped
                    </Badge>
                  )}
                </div>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="workspace-name">Name</Label>
                    <Input
                      id="workspace-name"
                      value={name}
                      onChange={(event) => setName(event.target.value)}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Storage</Label>
                    <Select value={storageId} onValueChange={setStorageId}>
                      <SelectTrigger aria-label="Workspace storage">
                        <SelectValue placeholder="Choose storage" />
                      </SelectTrigger>
                      <SelectContent>
                        {storages.map((storage) => (
                          <SelectItem key={storage.id} value={storage.id}>
                            {storage.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <Label>Template</Label>
                    <Select
                      value={templateId}
                      onValueChange={(value) => setTemplateId(value as AgentWorkspaceTemplateId)}
                    >
                      <SelectTrigger aria-label="Workspace template">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {AGENT_WORKSPACE_TEMPLATES.map((template) => (
                          <SelectItem key={template.id} value={template.id}>
                            {template.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="workspace-root">Root path</Label>
                    <Input
                      id="workspace-root"
                      className="font-mono"
                      value={rootPath}
                      onChange={(event) => setRootPath(event.target.value)}
                    />
                  </div>
                </div>
                <div className="mt-4 flex flex-col gap-3 rounded-lg bg-muted/40 p-3 sm:flex-row sm:items-center sm:justify-between">
                  <div>
                    <div className="text-sm font-medium">Apply workspace MCP policy</div>
                    <div className="text-xs text-muted-foreground">
                      Sets default access to none and allows only the workspace root.
                    </div>
                  </div>
                  <Switch checked={applyPolicy} onCheckedChange={setApplyPolicy} />
                </div>
                <div className="mt-4 flex justify-end">
                  <Button onClick={handleCreate} disabled={!canCreate || isCreating}>
                    {isCreating ? <RefreshCw className="mr-2 h-4 w-4 animate-spin" /> : <Save className="mr-2 h-4 w-4" />}
                    Create workspace
                  </Button>
                </div>
              </section>

              {selectedWorkspace ? (
                <section className="grid gap-4 lg:grid-cols-[1fr_280px]">
                  <div className="rounded-xl border bg-background p-4">
                    <div className="mb-3 flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-sm font-medium">{selectedWorkspace.name}</h3>
                        <p className="font-mono text-xs text-muted-foreground">
                          {selectedWorkspaceStorage?.name ?? "Unknown storage"}:{selectedWorkspace.rootPath}
                        </p>
                      </div>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          onSelectStorage(selectedWorkspace.storageId);
                          onOpenChange(false);
                        }}
                      >
                        Open storage
                      </Button>
                    </div>
                    <div className="mb-3 flex flex-wrap gap-2">
                      <Badge variant="outline" className="gap-1">
                        <CheckCircle2 className="h-3 w-3" /> {selectedWorkspace.rootPath}
                      </Badge>
                      <Badge variant="outline">default access: none</Badge>
                    </div>

                    <div className="grid gap-3 md:grid-cols-[180px_1fr]">
                      <div className="space-y-1">
                        {memoryFiles.map((path) => (
                          <button
                            key={path}
                            type="button"
                            className={cn(
                              "w-full rounded-md px-2 py-1.5 text-left font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/30",
                              selectedMemoryFile === path
                                ? "bg-muted text-foreground"
                                : "text-muted-foreground hover:bg-muted/70",
                            )}
                            onClick={() => setSelectedMemoryFile(path)}
                          >
                            {path}
                          </button>
                        ))}
                      </div>
                      <div className="space-y-3">
                        <Textarea
                          value={memoryContent}
                          readOnly
                          className="min-h-44 font-mono text-xs"
                          aria-label="Memory file contents"
                        />
                        <Textarea
                          value={memoryAppendText}
                          onChange={(event) => setMemoryAppendText(event.target.value)}
                          placeholder="Append a note to the selected memory file..."
                          className="min-h-20"
                          aria-label="Memory note"
                        />
                        <div className="flex justify-end">
                          <Button
                            variant="outline"
                            onClick={handleAppendMemory}
                            disabled={isMemoryBusy || !memoryAppendText.trim()}
                          >
                            {isMemoryBusy ? (
                              <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
                            ) : (
                              <NotebookPen className="mr-2 h-4 w-4" />
                            )}
                            Append memory
                          </Button>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div className="rounded-xl border bg-background p-4">
                    <h3 className="text-sm font-medium">Checkpoints</h3>
                    <p className="mt-1 text-xs text-muted-foreground">
                      Capture memory files locally and under `.infimount/checkpoints` in the workspace.
                    </p>
                    <Button
                      className="mt-4 w-full"
                      variant="outline"
                      onClick={handleCheckpoint}
                      disabled={isCheckpointBusy}
                    >
                      {isCheckpointBusy ? (
                        <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
                      ) : (
                        <Save className="mr-2 h-4 w-4" />
                      )}
                      Save checkpoint
                    </Button>
                    <div className="mt-4 space-y-2">
                      <Label>Restore point</Label>
                      <Select value={selectedCheckpointId} onValueChange={setSelectedCheckpointId}>
                        <SelectTrigger aria-label="Workspace checkpoint">
                          <SelectValue placeholder="Choose checkpoint" />
                        </SelectTrigger>
                        <SelectContent>
                          {checkpoints.map((checkpoint) => (
                            <SelectItem key={checkpoint.id} value={checkpoint.id}>
                              {new Date(checkpoint.createdAt).toLocaleString()}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <Button
                      className="mt-3 w-full"
                      variant="secondary"
                      onClick={() => setRestoreConfirmationOpen(true)}
                      disabled={isCheckpointBusy || !selectedCheckpointId}
                    >
                      <RotateCcw className="mr-2 h-4 w-4" />
                      Restore memory
                    </Button>

                    <div className="mt-5 space-y-2 border-t pt-4">
                      <div className="text-sm font-medium text-destructive">Remove workspace</div>
                      <Button
                        className="w-full"
                        variant="outline"
                        onClick={() => setDeleteMode("registration")}
                      >
                        Remove registration only
                      </Button>
                      <Button
                        className="w-full"
                        variant="destructive"
                        onClick={() => setDeleteMode("files")}
                      >
                        <Trash2 className="mr-2 h-4 w-4" />
                        Delete registration and files
                      </Button>
                    </div>

                    <div className="mt-5 border-t pt-4">
                      <div className="flex items-center gap-2 text-sm font-medium">
                        <Clock3 className="h-4 w-4" />
                        Workspace audit
                      </div>
                      <div className="mt-3 space-y-2">
                        {workspaceAuditItems.length === 0 ? (
                          <p className="text-xs text-muted-foreground">
                            No workspace activity recorded yet.
                          </p>
                        ) : (
                          workspaceAuditItems.slice(0, 6).map((item) => (
                            <div key={item.id} className="rounded-lg bg-muted/40 p-2">
                              <div className="text-xs font-medium">{item.title}</div>
                              <div className="mt-0.5 text-[11px] text-muted-foreground">
                                {new Date(item.createdAt).toLocaleString()}
                              </div>
                              {item.detail ? (
                                <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
                                  {item.detail}
                                </div>
                              ) : null}
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  </div>
                </section>
              ) : null}
            </div>
          </ScrollArea>
        </div>

        <AlertDialog
          open={restoreConfirmationOpen}
          onOpenChange={(next) => !isCheckpointBusy && setRestoreConfirmationOpen(next)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Overwrite workspace memory?</AlertDialogTitle>
              <AlertDialogDescription>
                Restoring {selectedCheckpointId || "this checkpoint"} will replace every managed memory file in {selectedWorkspace?.name ?? "this workspace"}. Current contents will be lost.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={isCheckpointBusy}>Cancel</AlertDialogCancel>
              <AlertDialogAction
                disabled={isCheckpointBusy || !selectedWorkspace || !selectedCheckpointId}
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                onClick={(event) => {
                  event.preventDefault();
                  void handleRestore();
                }}
              >
                {isCheckpointBusy ? "Restoring…" : "Restore and overwrite"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <AlertDialog open={deleteMode !== null} onOpenChange={(next) => !next && setDeleteMode(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {deleteMode === "files" ? "Delete workspace files permanently?" : "Remove workspace registration?"}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {deleteMode === "files"
                  ? `This permanently deletes ${selectedWorkspace?.rootPath ?? "the workspace root"} and all files below it, then removes its policy and registration. This cannot be undone.`
                  : "This removes the workspace policy and registration but preserves every storage file."}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={isDeleting}>Cancel</AlertDialogCancel>
              <AlertDialogAction
                disabled={isDeleting}
                className={deleteMode === "files" ? "bg-destructive text-destructive-foreground hover:bg-destructive/90" : undefined}
                onClick={(event) => {
                  event.preventDefault();
                  void handleDeleteWorkspace();
                }}
              >
                {isDeleting
                  ? "Deleting…"
                  : deleteMode === "files"
                    ? "Delete registration and files"
                    : "Remove registration only"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </DialogContent>
    </Dialog>
  );
}

interface WorkspaceAuditItem {
  id: string;
  createdAt: number;
  title: string;
  detail?: string;
}

function buildWorkspaceAuditItems(
  workspace: AgentWorkspace,
  activityEvents: ActivityLogEvent[],
  auditEvents: McpAuditEvent[],
): WorkspaceAuditItem[] {
  const activityItems = activityEvents
    .filter((event) => event.workspaceId === workspace.id)
    .map((event) => ({
      id: event.id,
      createdAt: event.createdAt,
      title: event.message ?? humanizeActivityType(event.type),
      detail: typeof event.summary?.rootPath === "string" ? event.summary.rootPath : undefined,
    }));

  const mcpItems = auditEvents
    .filter((event) => event.storage_id === workspace.storageId)
    .filter((event) => isWorkspacePath(workspace.rootPath, event.path))
    .map((event) => ({
      id: event.id,
      createdAt: Date.parse(event.timestamp),
      title: `MCP ${event.operation}: ${event.decision}`,
      detail: event.path ?? undefined,
    }));

  return [...activityItems, ...mcpItems]
    .filter((event) => Number.isFinite(event.createdAt))
    .sort((a, b) => b.createdAt - a.createdAt);
}

function isWorkspacePath(rootPath: string, path: string | null): boolean {
  if (!path) return false;
  const root = rootPath.endsWith("/") ? rootPath : `${rootPath}/`;
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  return normalizedPath === rootPath || normalizedPath.startsWith(root);
}

function humanizeActivityType(type: ActivityLogEvent["type"]): string {
  return type
    .replace(/^workspace_/, "")
    .replace(/_/g, " ")
    .replace(/^./, (character) => character.toUpperCase());
}
