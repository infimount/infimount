import { useCallback, useEffect, useRef, useState } from "react";
import { FileClock, RefreshCw, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  listHostedTaskExecutionStatus,
  reconcileDeletedHostedAttempt,
  cleanupPinnedHostedSession,
  hostedPilotBuildInfo,
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
    case "public_session_deleted_needs_local_review":
      return "The provider confirmed removal from its public API. Local import or terminal evidence still needs inspection. Physical erasure is not confirmed.";
    case "public_session_deletion_confirmed":
      return "The provider confirmed removal from its public API. Physical erasure and retention expiry are not confirmed.";
    case "terminal_needs_cleanup_review":
      return "Execution is terminal. The private record does not prove provider-side deletion.";
  }
}

/** Private, read-only execution evidence. Never displays provider session IDs. */
export function HostedTaskRecoveryStatus({ workspaceId, taskId }: Props) {
  const [history, setHistory] = useState<HostedExecutionHistory | null>(null);
  const [loading, setLoading] = useState(false);
  const [reconciling, setReconciling] = useState<string | null>(null);
  const [cleaning, setCleaning] = useState<string | null>(null);
  const [nativePilotEnabled, setNativePilotEnabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  // Keep mutation lifetime separate from read generations. Switching tasks
  // invalidates result rendering, not the native recovery operation.
  const actionInFlight = useRef(false);
  const actionBusy = cleaning !== null || reconciling !== null;

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
    let alive = true;
    setNativePilotEnabled(false);
    void hostedPilotBuildInfo().then((info) => {
      if (alive) setNativePilotEnabled(info.enabled && info.syntheticOnly);
    });
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    setHistory(null);
    setError(null);
    void refresh();
    return () => { generation.current += 1; };
  }, [refresh]);

  const reconcile = useCallback(async (executionId: string) => {
    if (loading || actionInFlight.current) return;
    const current = generation.current;
    actionInFlight.current = true;
    setReconciling(executionId);
    setError(null);
    try {
      const resolved = await reconcileDeletedHostedAttempt({ workspaceId, taskId, executionId });
      if (generation.current !== current) return;
      if (resolved !== true) throw new Error("Local reconciliation not confirmed");
      await refresh();
    } catch {
      if (generation.current !== current) return;
      setError("Local reconciliation is blocked. Preserve existing outputs and inspect the execution status.");
    } finally {
      actionInFlight.current = false;
      setReconciling(null);
    }
  }, [loading, refresh, taskId, workspaceId]);

  const cleanup = useCallback(async (executionId: string) => {
    if (!nativePilotEnabled || loading || actionInFlight.current) return;
    const current = generation.current;
    actionInFlight.current = true;
    setCleaning(executionId);
    setError(null);
    try {
      const deleted = await cleanupPinnedHostedSession({ workspaceId, taskId, executionId });
      if (generation.current !== current) return;
      if (deleted !== true) throw new Error("Provider deletion not confirmed");
      await refresh();
    } catch {
      if (generation.current !== current) return;
      setError("Authenticated cleanup was not confirmed. The same session remains unresolved. Do not start another run.");
    } finally {
      actionInFlight.current = false;
      setCleaning(null);
    }
  }, [nativePilotEnabled, loading, refresh, workspaceId, taskId]);

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
        <Button
          variant="outline"
          size="sm"
          disabled={loading || actionBusy}
          onClick={() => { if (!actionInFlight.current) void refresh(); }}
        >
          <RefreshCw className={`mr-2 h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          Refresh status
        </Button>
      </div>
      {actionBusy ? (
        <p aria-live="polite" className="mt-3 text-xs text-muted-foreground">
          A native recovery action is in progress. Changing tasks does not cancel it.
        </p>
      ) : null}
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
          {attempt.recovery === "inspect_pinned_session" && nativePilotEnabled ? (
            <Button
              className="mt-2"
              variant="outline"
              size="sm"
              disabled={loading || actionBusy}
              onClick={() => void cleanup(attempt.executionId)}
            >
              {cleaning === attempt.executionId ? "Cleaning provider session…" : "Clean up pinned provider session"}
            </Button>
          ) : null}
          {attempt.recovery === "requires_fresh_approval" ? (
            <Button
              className="mt-2"
              variant="outline"
              size="sm"
              disabled={loading || actionBusy}
              onClick={() => void reconcile(attempt.executionId)}
            >
              {reconciling === attempt.executionId ? "Checking local state…" : "Discard unsubmitted attempt"}
            </Button>
          ) : null}
          {attempt.recovery === "public_session_deleted_needs_local_review" ? (
            <Button
              className="mt-2"
              variant="outline"
              size="sm"
              disabled={loading || actionBusy}
              onClick={() => void reconcile(attempt.executionId)}
            >
              {reconciling === attempt.executionId ? "Checking local state…" : "Resolve local attempt"}
            </Button>
          ) : null}
        </article>
      ))}
    </section>
  );
}
