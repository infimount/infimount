import { useEffect, useMemo, useState } from "react";
import { Cable, CheckCircle2, CircleAlert, Copy, Play, ShieldCheck, Square } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { summarizeAgentAccess } from "@/lib/agentAccessStatus";
import type { WorkspaceRecord } from "@/lib/api";
import type { McpClientSnippets, McpRuntimeStatus, StorageConfig } from "@/types/storage";

interface AgentAccessCenterProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: McpRuntimeStatus | null;
  snippets: McpClientSnippets | null;
  workspaces: WorkspaceRecord[];
  storages: StorageConfig[];
  onPrepare: (workspace: WorkspaceRecord) => Promise<void>;
  onVerify: () => Promise<void>;
  onStartHttp: () => Promise<void>;
  onStopHttp: () => Promise<void>;
  onOpenAdvanced: () => void;
  onOpenWorkspaces: () => void;
}

export function AgentAccessCenter({
  open,
  onOpenChange,
  status,
  snippets,
  workspaces,
  storages,
  onPrepare,
  onVerify,
  onStartHttp,
  onStopHttp,
  onOpenAdvanced,
  onOpenWorkspaces,
}: AgentAccessCenterProps) {
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState("");
  const [busy, setBusy] = useState<"prepare" | "verify" | "http" | null>(null);
  const [lastVerification, setLastVerification] = useState<"passed" | "failed" | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!open) return;
    if (selectedWorkspaceId && workspaces.some((workspace) => workspace.id === selectedWorkspaceId)) {
      return;
    }
    setSelectedWorkspaceId(workspaces[0]?.id ?? "");
  }, [open, selectedWorkspaceId, workspaces]);

  useEffect(() => {
    if (!open) {
      setBusy(null);
      setLastVerification(null);
      setCopied(false);
    }
  }, [open]);

  const selectedWorkspace = useMemo(
    () => workspaces.find((workspace) => workspace.id === selectedWorkspaceId) ?? null,
    [selectedWorkspaceId, workspaces],
  );
  const selectedStorage = selectedWorkspace
    ? storages.find((storage) => storage.id === selectedWorkspace.storageId) ?? null
    : null;
  const summary = summarizeAgentAccess(status, workspaces, storages);
  const selectedPrepared = Boolean(
    selectedWorkspace &&
      selectedStorage?.enabled &&
      selectedStorage.mcpExposed &&
      status?.settings.enabled,
  );
  const selectedSnippet = status?.settings.transport === "http" ? snippets?.http : snippets?.stdio;

  const handlePrepare = async () => {
    if (!selectedWorkspace || busy) return;
    setBusy("prepare");
    setLastVerification(null);
    try {
      await onPrepare(selectedWorkspace);
    } finally {
      setBusy(null);
    }
  };

  const handleVerify = async () => {
    if (busy) return;
    setBusy("verify");
    try {
      await onVerify();
      setLastVerification("passed");
    } catch {
      setLastVerification("failed");
    } finally {
      setBusy(null);
    }
  };

  const handleHttpToggle = async () => {
    if (busy || status?.settings.transport !== "http") return;
    setBusy("http");
    try {
      if (status.runningHttp) {
        await onStopHttp();
      } else {
        await onStartHttp();
      }
    } finally {
      setBusy(null);
    }
  };

  const handleCopy = async () => {
    if (!selectedSnippet) return;
    await navigator.clipboard.writeText(selectedSnippet);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1800);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(90vh,760px)] flex-col overflow-hidden sm:max-w-[760px]">
        <DialogHeader className="shrink-0">
          <DialogTitle className="flex items-center gap-2">
            <Cable className="h-5 w-5" />
            Agent Access
          </DialogTitle>
          <DialogDescription>
            Connect an AI client to one scoped workspace. Storage browsing remains independent.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
          <section className="rounded-xl border bg-card p-4">
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="text-sm font-medium">Current status</div>
                <div className="mt-1 text-sm text-muted-foreground">{summary.detail}</div>
              </div>
              <div
                className={
                  summary.tone === "success"
                    ? "rounded-full bg-emerald-500/15 px-2.5 py-1 text-xs font-medium text-emerald-700 dark:text-emerald-300"
                    : summary.tone === "warning"
                      ? "rounded-full bg-amber-500/15 px-2.5 py-1 text-xs font-medium text-amber-700 dark:text-amber-300"
                      : "rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground"
                }
              >
                {summary.label}
              </div>
            </div>
          </section>

          {workspaces.length === 0 ? (
            <section className="rounded-xl border border-dashed p-5 text-center">
              <ShieldCheck className="mx-auto h-6 w-6 text-muted-foreground" />
              <h3 className="mt-3 text-sm font-medium">Create a workspace first</h3>
              <p className="mt-1 text-sm text-muted-foreground">
                A workspace is the storage boundary Infimount exposes to an agent.
              </p>
              <Button className="mt-4" onClick={onOpenWorkspaces}>
                Create workspace
              </Button>
            </section>
          ) : (
            <>
              <section className="space-y-3 rounded-xl border p-4">
                <div>
                  <div className="text-sm font-medium">Workspace</div>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Agent Access is prepared per workspace boundary.
                  </p>
                </div>
                <Select value={selectedWorkspaceId} onValueChange={setSelectedWorkspaceId}>
                  <SelectTrigger aria-label="Agent Access workspace">
                    <SelectValue placeholder="Choose workspace" />
                  </SelectTrigger>
                  <SelectContent>
                    {workspaces.map((workspace) => (
                      <SelectItem key={workspace.id} value={workspace.id}>
                        {workspace.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {selectedWorkspace ? (
                  <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
                    <div className="rounded-lg bg-muted/40 p-3">
                      <div className="font-medium text-foreground">Access profile</div>
                      <div className="mt-1">
                        {selectedWorkspace.accessProfile === "read_write" ? "Read/write" : "Read-only"}
                      </div>
                    </div>
                    <div className="rounded-lg bg-muted/40 p-3">
                      <div className="font-medium text-foreground">Storage boundary</div>
                      <div className="mt-1">{selectedStorage?.name ?? "Storage unavailable"}</div>
                    </div>
                  </div>
                ) : null}
                <div className="flex flex-wrap gap-2">
                  <Button onClick={() => void handlePrepare()} disabled={!selectedWorkspace || busy !== null}>
                    {busy === "prepare"
                      ? "Preparing…"
                      : selectedPrepared
                        ? "Re-check agent access"
                        : "Prepare agent access"}
                  </Button>
                  <Button variant="outline" onClick={onOpenWorkspaces} disabled={busy !== null}>
                    Manage workspaces
                  </Button>
                </div>
              </section>

              <section className="rounded-xl border p-4">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <div className="text-sm font-medium">Connection</div>
                    {status?.settings.transport === "stdio" ? (
                      <p className="mt-1 text-sm text-muted-foreground">
                        stdio is on demand. No background server is required. Your MCP client launches
                        the bundled Infimount sidecar when it connects.
                      </p>
                    ) : (
                      <p className="mt-1 text-sm text-muted-foreground">
                        HTTP uses a persistent local server. Start it before an HTTP client connects.
                      </p>
                    )}
                  </div>
                  <div className="rounded-full bg-muted px-2.5 py-1 text-xs font-medium">
                    {status?.settings.transport ?? "unknown"}
                  </div>
                </div>

                {status?.settings.transport === "http" ? (
                  <div className="mt-4 space-y-3">
                    <div className="rounded-lg bg-muted/40 p-3 font-mono text-xs">
                      {status.endpointDisplay}
                    </div>
                    <Button onClick={() => void handleHttpToggle()} disabled={!selectedPrepared || busy !== null}>
                      {status.runningHttp ? (
                        <Square className="mr-2 h-4 w-4" />
                      ) : (
                        <Play className="mr-2 h-4 w-4" />
                      )}
                      {busy === "http"
                        ? "Working…"
                        : status.runningHttp
                          ? "Stop HTTP server"
                          : "Start HTTP server"}
                    </Button>
                  </div>
                ) : null}
              </section>

              <section className="space-y-3 rounded-xl border p-4">
                <div>
                  <div className="text-sm font-medium">Connect client</div>
                  <p className="mt-1 text-xs text-muted-foreground">
                    The configuration below matches the selected transport. Advanced settings remain
                    available separately.
                  </p>
                </div>
                <Textarea
                  readOnly
                  rows={8}
                  value={selectedSnippet ?? "Prepare Agent Access to load the client configuration."}
                  className="font-mono text-xs"
                  aria-label="Selected MCP client configuration"
                />
                <Button variant="outline" onClick={() => void handleCopy()} disabled={!selectedSnippet}>
                  <Copy className="mr-2 h-4 w-4" />
                  {copied ? "Copied" : "Copy client config"}
                </Button>
              </section>

              <section className="rounded-xl border p-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <div className="text-sm font-medium">Verify</div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      Run the packaged sidecar and policy-denial probe before relying on this connection.
                    </p>
                  </div>
                  <Button variant="outline" onClick={() => void handleVerify()} disabled={!selectedPrepared || busy !== null}>
                    {busy === "verify" ? "Verifying…" : "Verify connection"}
                  </Button>
                </div>
                {lastVerification ? (
                  <div
                    className={
                      lastVerification === "passed"
                        ? "mt-3 flex items-center gap-2 text-sm text-emerald-700 dark:text-emerald-300"
                        : "mt-3 flex items-center gap-2 text-sm text-destructive"
                    }
                  >
                    {lastVerification === "passed" ? (
                      <CheckCircle2 className="h-4 w-4" />
                    ) : (
                      <CircleAlert className="h-4 w-4" />
                    )}
                    {lastVerification === "passed"
                      ? "Verification passed."
                      : "Verification failed. Review Agent Access or Advanced MCP settings."}
                  </div>
                ) : null}
              </section>
            </>
          )}
        </div>

        <div className="flex shrink-0 justify-end border-t pt-3">
          <Button variant="ghost" onClick={onOpenAdvanced}>
            Advanced MCP settings
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
