import {
  CheckCircle2,
  Clock3,
  Copy,
  RotateCcw,
  Trash2,
  X,
  XCircle,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";
import { useTransferQueue, type TransferJob } from "@/hooks/use-transfer-queue";

const STATUS_LABELS: Record<TransferJob["status"], string> = {
  queued: "Queued",
  running: "Running",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

function formatJobTitle(job: TransferJob) {
  const verb = job.operation === "copy" ? "Copy" : "Move";
  const count = job.paths.length;
  return `${verb} ${count} item${count === 1 ? "" : "s"}`;
}

function formatRoute(job: TransferJob) {
  const from = job.sourceName ?? job.fromSourceId;
  const to = job.destinationName ?? job.toSourceId;
  return `${from} → ${to}`;
}

function statusIcon(job: TransferJob) {
  switch (job.status) {
    case "completed":
      return <CheckCircle2 className="h-4 w-4 text-emerald-600" aria-hidden="true" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" aria-hidden="true" />;
    case "cancelled":
      return <X className="h-4 w-4 text-muted-foreground" aria-hidden="true" />;
    case "queued":
      return <Clock3 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />;
    case "running":
    default:
      return <Copy className="h-4 w-4 text-foreground" aria-hidden="true" />;
  }
}

export function TransferQueuePanel() {
  const {
    jobs,
    activeJob,
    retryTransfer,
    cancelTransfer,
    clearCompletedTransfers,
    clearTransfer,
  } = useTransferQueue();

  const visibleJobs = jobs.filter((job) => job.status !== "cancelled").slice(-5).reverse();
  const failedCount = jobs.filter((job) => job.status === "failed").length;
  const queuedCount = jobs.filter((job) => job.status === "queued").length;

  if (jobs.length === 0) return null;

  return (
    <section
      className="absolute bottom-12 right-4 z-20 w-[360px] max-w-[calc(100%-2rem)] rounded-xl border border-border bg-card text-card-foreground shadow-lg"
      aria-label="Transfer queue"
    >
      <div className="flex items-center justify-between gap-3 border-b border-border/70 px-3 py-2">
        <div>
          <h2 className="text-sm font-medium leading-tight">Transfer queue</h2>
          <p className="text-[11px] text-muted-foreground">
            {activeJob ? STATUS_LABELS[activeJob.status] : "Idle"}
            {queuedCount > 0 ? `, ${queuedCount} queued` : ""}
            {failedCount > 0 ? `, ${failedCount} failed` : ""}
          </p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={clearCompletedTransfers}
          disabled={!jobs.some((job) => job.status === "completed" || job.status === "cancelled")}
        >
          Clear done
        </Button>
      </div>

      <div className="max-h-72 overflow-y-auto p-2">
        {visibleJobs.length === 0 ? (
          <div className="px-2 py-6 text-center text-xs text-muted-foreground">
            No active transfers.
          </div>
        ) : (
          <div className="space-y-2">
            {visibleJobs.map((job) => (
              <article
                key={job.id}
                className={cn(
                  "rounded-lg border border-border/70 bg-background px-3 py-2",
                  job.status === "failed" && "border-destructive/30 bg-destructive/5",
                )}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      {statusIcon(job)}
                      <span className="truncate text-sm font-medium">{formatJobTitle(job)}</span>
                      <span className="rounded-full border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {STATUS_LABELS[job.status]}
                      </span>
                    </div>
                    <p className="mt-1 truncate text-[11px] text-muted-foreground">
                      {formatRoute(job)} · {job.targetDir}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {job.status === "queued" || job.status === "running" ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        aria-label={job.status === "running" ? "Cancel active transfer" : "Cancel queued transfer"}
                        onClick={() => cancelTransfer(job.id)}
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    ) : null}
                    {job.status === "failed" ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        aria-label="Retry transfer"
                        onClick={() => retryTransfer(job.id)}
                      >
                        <RotateCcw className="h-3.5 w-3.5" />
                      </Button>
                    ) : null}
                    {job.status === "completed" || job.status === "failed" ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        aria-label="Remove transfer"
                        onClick={() => clearTransfer(job.id)}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    ) : null}
                  </div>
                </div>

                <Progress
                  value={job.status === "queued" ? 0 : job.progress}
                  className="mt-2 h-1.5 bg-muted"
                  aria-label={`${formatJobTitle(job)} progress`}
                />
                {job.currentPath && job.status === "running" ? (
                  <p className="mt-2 truncate text-[11px] text-muted-foreground">
                    Current: {job.currentPath}
                  </p>
                ) : null}
                {job.error ? (
                  <p className="mt-2 line-clamp-2 text-[11px] text-destructive">{job.error}</p>
                ) : null}
              </article>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
