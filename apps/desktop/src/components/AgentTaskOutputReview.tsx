import { useEffect, useRef, useState } from "react";
import { FileSearch, RefreshCw } from "lucide-react";

import { AgentTaskPublicationPanel } from "./AgentTaskPublicationPanel";
import { Button } from "@/components/ui/button";
import {
  reviewAgentTaskOutputs,
  type AgentTaskOutputReviewOutput,
} from "@/lib/agentTasks";

interface AgentTaskOutputReviewProps {
  workspaceId: string;
  taskId: string;
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function previewReason(reason: string | null): string {
  if (reason === "too_large") return "Text preview is disabled above 64 KiB; integrity fingerprint is still available.";
  if (reason === "binary") return "Binary or control-character content is not rendered as text.";
  if (reason === "preview_limit") return "Inline preview is limited to the first 20 output files; integrity fingerprint is still available.";
  return "Text preview is unavailable.";
}

export function AgentTaskOutputReview({ workspaceId, taskId }: AgentTaskOutputReviewProps) {
  const [review, setReview] = useState<AgentTaskOutputReviewOutput | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestKey = `${workspaceId}:${taskId}`;
  const currentRequestKey = useRef(requestKey);
  currentRequestKey.current = requestKey;

  useEffect(() => {
    setReview(null);
    setRunning(false);
    setError(null);
  }, [requestKey]);

  const refresh = async () => {
    if (running) return;
    const startedFor = requestKey;
    setRunning(true);
    setError(null);
    try {
      const result = await reviewAgentTaskOutputs({ workspaceId, taskId });
      if (currentRequestKey.current !== startedFor) return;
      setReview(result);
    } catch (value) {
      if (currentRequestKey.current !== startedFor) return;
      setError(value instanceof Error ? value.message : "Agent Task output review failed.");
    } finally {
      if (currentRequestKey.current === startedFor) setRunning(false);
    }
  };

  return (
    <section className="space-y-3 rounded-xl border bg-muted/15 p-4" data-testid="agent-task-output-review-section">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 font-medium">
            <FileSearch className="h-4 w-4 text-primary" />
            Output review
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Read the current task outputs and capture fresh SHA-256 fingerprints. Nothing is published until you explicitly select outputs and approve a separate publication plan.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void refresh()} disabled={running}>
          <RefreshCw className={`mr-2 h-4 w-4 ${running ? "animate-spin" : ""}`} />
          {running ? "Reviewing…" : review ? "Refresh outputs" : "Review outputs"}
        </Button>
      </div>

      {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}

      {review ? (
        <div className="space-y-3" data-testid="agent-task-output-review">
          <div className="text-sm text-muted-foreground">
            {review.fileCount} output file{review.fileCount === 1 ? "" : "s"} · {formatBytes(review.totalBytes)}
          </div>
          {review.files.length === 0 ? (
            <div className="rounded-md border border-dashed px-3 py-4 text-sm text-muted-foreground">
              No output files yet. Finish the Codex task, then refresh this review.
            </div>
          ) : (
            <>
              <div className="space-y-3">
                {review.files.map((file) => (
                  <article key={`${file.taskPath}:${file.sha256}`} className="rounded-lg border bg-background p-3">
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <code className="break-all text-xs font-medium">{file.taskPath}</code>
                      <span className="text-xs text-muted-foreground">{formatBytes(file.byteSize)}</span>
                    </div>
                    <div className="mt-2 break-all font-mono text-[11px] text-muted-foreground" title={file.sha256}>
                      SHA-256 {file.sha256}
                    </div>
                    {file.preview !== null ? (
                      <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/40 p-3 text-xs">
                        {file.preview}
                      </pre>
                    ) : (
                      <p className="mt-3 text-xs text-muted-foreground">
                        {previewReason(file.previewUnavailableReason)}
                      </p>
                    )}
                  </article>
                ))}
              </div>
              <AgentTaskPublicationPanel review={review} />
            </>
          )}
        </div>
      ) : null}
    </section>
  );
}
