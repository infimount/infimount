import { useEffect, useRef, useState } from "react";
import { FileCheck2, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  previewHostedTaskExport,
  type HostedTaskExportPreview,
  type PreviewHostedTaskExportRequest,
} from "@/lib/agentTasks";

interface Props {
  workspaceId: string;
  taskId: string;
}

function requestedOutputsFromText(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((path) => (path.startsWith("outputs/") ? path : `outputs/${path}`));
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/**
 * A read-only scope preview. Never calls a provider, uploads files, stores
 * credentials, mints an approval, or initiates a paid session.
 */
export function AgentTaskHostedExportPreview({ workspaceId, taskId }: Props) {
  const [opened, setOpened] = useState(false);
  const [reviewedTitle, setReviewedTitle] = useState("");
  const [objective, setObjective] = useState("");
  const [outputsText, setOutputsText] = useState("");
  const [preview, setPreview] = useState<HostedTaskExportPreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // A monotonic generation also rejects ABA edits (A -> B -> A) while an
  // older preview is in flight. A matching text key alone is not sufficient.
  const generation = useRef(0);

  const invalidateReview = () => {
    generation.current += 1;
    setPreview(null);
    setError(null);
    setLoading(false);
  };

  useEffect(() => {
    generation.current += 1;
    setOpened(false);
    setReviewedTitle("");
    setObjective("");
    setOutputsText("");
    setPreview(null);
    setLoading(false);
    setError(null);
  }, [workspaceId, taskId]);

  const reviewedOutputs = requestedOutputsFromText(outputsText);
  const valid = reviewedTitle.trim().length > 0 && objective.trim().length > 0 && reviewedOutputs.length > 0;

  const showScope = async () => {
    if (!valid || loading) return;
    const currentGeneration = ++generation.current;
    const request: PreviewHostedTaskExportRequest = {
      workspaceId,
      taskId,
      reviewedExportBrief: {
        title: reviewedTitle.trim(),
        objective: objective.trim(),
        requestedOutputs: reviewedOutputs,
      },
    };
    setLoading(true);
    setError(null);
    setPreview(null);
    try {
      const result = await previewHostedTaskExport(request);
      if (generation.current !== currentGeneration) return;
      setPreview(result);
    } catch (value) {
      if (generation.current !== currentGeneration) return;
      setError(value instanceof Error ? value.message : "Remote export preview failed.");
    } finally {
      if (generation.current === currentGeneration) setLoading(false);
    }
  };

  return (
    <section className="mt-3 rounded-xl border bg-muted/15 p-4" data-testid="hosted-export-review">
      <button
        type="button"
        className="flex w-full items-center justify-between text-left text-sm font-medium"
        aria-expanded={opened}
        onClick={() => {
          if (opened) invalidateReview();
          setOpened(!opened);
        }
      >
        <span className="flex items-center gap-2">
          <FileCheck2 className="h-4 w-4 text-primary" />
          Review remote sandbox export
        </span>
        <span className="text-xs text-muted-foreground">{opened ? "Hide" : "Show"}</span>
      </button>
      {opened ? (
        <div className="mt-3 space-y-3" data-testid="hosted-export-form">
          <div className="flex gap-2 rounded-md border p-3 text-xs text-muted-foreground">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              This is a metadata-only preview, not an upload or consent.
              OpenAI Agents API currently stores session data in the United States and does not offer Zero Data Retention.
              Disabling sandbox network egress does not change provider residency or retention.
            </span>
          </div>
          <p className="text-xs text-muted-foreground">
            Enter a separately reviewed, redacted title, objective, and output paths. The original TASK.md is never copied automatically.
          </p>
          <label className="block space-y-1 text-xs">
            <span>Reviewed export title</span>
            <Input
              aria-label="Reviewed export title"
              value={reviewedTitle}
              onChange={(event) => {
                invalidateReview();
                setReviewedTitle(event.target.value);
              }}
              maxLength={120}
              placeholder="Example: Summarize approved sample data"
            />
          </label>
          <label className="block space-y-1 text-xs">
            <span>Reviewed objective</span>
            <Textarea
              aria-label="Reviewed objective"
              value={objective}
              onChange={(event) => {
                invalidateReview();
                setObjective(event.target.value);
              }}
              rows={3}
              maxLength={32768}
              placeholder="Describe only what the remote agent may do with the reviewed input copies."
            />
          </label>
          <label className="block space-y-1 text-xs">
            <span>Requested outputs, one path per line</span>
            <Textarea
              aria-label="Requested outputs"
              value={outputsText}
              onChange={(event) => {
                invalidateReview();
                setOutputsText(event.target.value);
              }}
              rows={3}
              placeholder="outputs/summary.md"
            />
          </label>
          <Button variant="outline" size="sm" onClick={() => void showScope()} disabled={!valid || loading}>
            {loading ? "Checking scope…" : "Preview export scope (no upload)"}
          </Button>
          {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
          {preview ? (
            <div className="space-y-3 rounded-md border bg-background p-3 text-xs" data-testid="hosted-export-preview">
              <div className="font-medium">
                {preview.inputFileCount} declared input file{preview.inputFileCount === 1 ? "" : "s"}
                {" · "}{formatBytes(preview.inputTotalBytes)} · ${(preview.pilotSpendLimitUsdCents / 100).toFixed(2)} USD ceiling
              </div>
              <p className="text-muted-foreground">
                Sandbox outbound network: disabled. Provider data residency: United States.
                Zero Data Retention: not supported. File hashes below are manifest declarations, not freshly verified bytes.
              </p>
              <div className="space-y-2">
                {preview.inputs.map((input) => (
                  <div key={input.taskPath} className="rounded border p-2">
                    <code className="break-all">{input.taskPath}</code>
                    <div className="mt-1 text-muted-foreground">{formatBytes(input.byteSize)}</div>
                    <div className="break-all font-mono text-[10px]">SHA-256 {input.sha256}</div>
                  </div>
                ))}
              </div>
              <div>Requested deliverables: {preview.requestedOutputs.join(", ")}</div>
              <div className="break-all font-mono text-[10px]">
                Manifest-only export digest: {preview.bundleMetadataSha256}
              </div>
              <p className="font-medium">
                No data transferred. No spending or publication authorized. A future execution must rehash input bytes and obtain separate consent.
              </p>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
