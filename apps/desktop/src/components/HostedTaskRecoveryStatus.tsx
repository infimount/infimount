import { useCallback, useEffect, useRef, useState } from "react";
import { FileClock, RefreshCw, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  listHostedTaskExecutionStatus,
  type HostedExecutionHistory,
  type HostedExecutionHistoryItem,
} from "@/lib/agentTasks";

interface Props {
  workspaceId: string;
  taskId: string;
}

function recoveryMessage(item: HostedExecutionHistoryItem): string {
  switch (item.recovery) {
    case "requires_fresh_approval":
      return "Prepared only. A fresh user-approved execution is required.";
    case "manual_session_reconciliation":
      return "Session creation may have succeeded, but its ID was not captured. Check provider sessions manually. Do not retry creation.";
    case "inspect_pinned_session":
      return "A provider session ID is stored privately. Inspect and clean up this same session before doing more work.";
    case "terminal_needs_cleanup_review":
      return "Execution is terminal. The private record does not prove provider-side deletion.";
  }
}

/** Private, read-only execution evidence. Never displays provider session IDs. */
export function HostedTaskRecoveryStatus({ workspaceId, taskId }: Props) {
  const [history, setHistory] = useState<HostedExecutionHistory | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++generation.current;
    setLoading(true);
    setError(null);
    try {
      const value = await listHostedTaskExecutionStatus({ workspaceId, taskId });
      if (id !== generation.current) return;
      setHistory(value);
    } catch {
      if (id !== generation.current) return;
      setError("Private hosted execution history is unavailable.");
      setHistory(null);
    } finally {
      if (id === generation.current) setLoading(false);
    }
  }, [taskId, workspaceId]);

  useEffect(() => {
    setHistory(null);
    setError(null);
    void refresh();
    return () => { generation.current += 1; };
  }, [refresh]);

  return (
    <section className="mt-3 rounded-xl border bg-muted/15 p-4" data-testid="hosted-execution-status">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium">
            <FileClock className="h-4 w-4" />
            Hosted execution history
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Private saved evidence only. Refresh does not contact OpenAI, retry a session, or authorize publication.
          </p>
        </div>
        <Button variant="outline" size="sm" disabled={loading} onClick={() => void refresh()}>
          <RefreshCw className={`mr-2 h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          Refresh status
        </Button>
      </div>
      {error ? <p role="alert" className="mt-3 text-xs text-destructive">{error}</p> : null}
      {!error && history?.attempts.length === 0 ? (
        <p className="mt-3 text-xs text-muted-foreground">No hosted execution has been recorded for this task.</p>
      ) : null}
      {!error && history?.attempts.map((attempt) => (
        <article key={attempt.executionId} className="mt-3 rounded-md border bg-background p-3">
          <div className="text-xs font-medium">
            {attempt.status} · {new Date(attempt.startedAt).toLocaleString()}
          </div>
          <p className="mt-1 flex gap-2 text-xs text-muted-foreground">
            {attempt.recovery === "manual_session_reconciliation" ? (
              <ShieldAlert className="h-4 w-4 shrink-0 text-amber-600" />
            ) : null}
            {recoveryMessage(attempt)}
          </p>
          <div className="mt-2 truncate font-mono text-[10px] text-muted-foreground">
            Attempt {attempt.executionId}
          </div>
        </article>
      ))}
    </section>
  );
}
