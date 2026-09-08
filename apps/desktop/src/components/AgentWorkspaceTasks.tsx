import { useCallback, useEffect, useRef, useState } from "react";
import { FileClock, RefreshCw, ShieldAlert } from "lucide-react";

import { AgentTaskOutputReview } from "./AgentTaskOutputReview";
import { Button } from "@/components/ui/button";
import {
  listAgentTasks,
  type AgentTaskListOutput,
  type AgentTaskSummary,
} from "@/lib/agentTasks";
import { cn } from "@/lib/utils";

interface AgentWorkspaceTasksProps {
  workspaceId: string;
}

export function AgentWorkspaceTasks({ workspaceId }: AgentWorkspaceTasksProps) {
  const [result, setResult] = useState<AgentTaskListOutput | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);

  const load = useCallback(async () => {
    const id = requestId.current + 1;
    requestId.current = id;
    setLoading(true);
    setError(null);
    try {
      const next = await listAgentTasks({ workspaceId });
      if (requestId.current !== id) return;
      setResult(next);
      setSelectedTaskId((current) =>
        current && next.tasks.some((task) => task.taskId === current)
          ? current
          : (next.tasks[0]?.taskId ?? null),
      );
    } catch (value) {
      if (requestId.current !== id) return;
      setResult(null);
      setSelectedTaskId(null);
      setError(value instanceof Error ? value.message : "Agent Tasks could not be loaded.");
    } finally {
      if (requestId.current === id) setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    setResult(null);
    setSelectedTaskId(null);
    setError(null);
    void load();
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  const selectedTask: AgentTaskSummary | null =
    result?.tasks.find((task) => task.taskId === selectedTaskId) ?? null;

  return (
    <section className="rounded-xl border bg-background p-4" data-testid="agent-workspace-tasks">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium">
            <FileClock className="h-4 w-4" />
            Agent Tasks
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Reopen committed tasks from this workspace to inspect outputs after the agent finishes or after Infimount restarts.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
          <RefreshCw className={`mr-2 h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          {loading ? "Loading…" : "Refresh tasks"}
        </Button>
      </div>

      {error ? <p role="alert" className="mt-3 text-sm text-destructive">{error}</p> : null}

      {result ? (
        <>
          {(result.truncated || result.skippedInvalidTasks > 0) ? (
            <div className="mt-3 flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-amber-800 dark:text-amber-300">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                {result.truncated
                  ? "The task list is bounded; older or excess entries may not be shown. "
                  : ""}
                {result.skippedInvalidTasks > 0
                  ? `${result.skippedInvalidTasks} malformed task ${result.skippedInvalidTasks === 1 ? "entry was" : "entries were"} ignored.`
                  : ""}
              </span>
            </div>
          ) : null}

          {result.tasks.length === 0 ? (
            <div className="mt-4 rounded-md border border-dashed px-3 py-5 text-sm text-muted-foreground">
              No committed Agent Tasks are available in this workspace yet.
            </div>
          ) : (
            <div className="mt-4 grid gap-4 lg:grid-cols-[240px_1fr]">
              <div className="max-h-80 space-y-1 overflow-auto pr-1" aria-label="Committed Agent Tasks">
                {result.tasks.map((task) => (
                  <button
                    key={task.taskId}
                    type="button"
                    className={cn(
                      "w-full rounded-lg border px-3 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/30",
                      selectedTaskId === task.taskId
                        ? "border-primary/30 bg-primary/5"
                        : "border-transparent bg-muted/30 hover:bg-muted/60",
                    )}
                    onClick={() => setSelectedTaskId(task.taskId)}
                  >
                    <div className="truncate text-sm font-medium">{task.title}</div>
                    <div className="mt-1 text-[11px] text-muted-foreground">
                      {new Date(task.createdAt).toLocaleString()}
                    </div>
                    <div className="mt-1 truncate font-mono text-[10px] text-muted-foreground">
                      {task.taskRoot}
                    </div>
                  </button>
                ))}
              </div>

              {selectedTask ? (
                <div className="min-w-0">
                  <div className="mb-2 text-xs text-muted-foreground">
                    Reviewing <strong className="text-foreground">{selectedTask.title}</strong>. Task discovery and review are read-only; publication remains a separate explicit step.
                  </div>
                  <AgentTaskOutputReview
                    workspaceId={workspaceId}
                    taskId={selectedTask.taskId}
                  />
                </div>
              ) : null}
            </div>
          )}
        </>
      ) : loading ? (
        <div className="mt-4 text-sm text-muted-foreground">Loading committed tasks…</div>
      ) : null}
    </section>
  );
}
