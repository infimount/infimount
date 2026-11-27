import { useEffect, useRef, useState } from "react";
import { FileCheck2, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  previewHostedTaskExport,
  verifyHostedTaskExport,
  hostedPilotBuildInfo,
  runHostedTaskPilot,
  type HostedPilotBuildInfo,
  type HostedTaskExportPreview,
  type PreviewHostedTaskExportRequest,
  type VerifiedHostedInputScope,
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
 * A read-only scope preview in ordinary builds. An explicitly compiled,
 * operator-only synthetic pilot adds a separate native-reviewed run action.
 * No webview boolean can approve spending or outbound file transfer.
 */
export function AgentTaskHostedExportPreview({ workspaceId, taskId }: Props) {
  const [opened, setOpened] = useState(false);
  const [reviewedTitle, setReviewedTitle] = useState("");
  const [objective, setObjective] = useState("");
  const [outputsText, setOutputsText] = useState("");
  const [preview, setPreview] = useState<HostedTaskExportPreview | null>(null);
  const [verified, setVerified] = useState<VerifiedHostedInputScope | null>(null);
  const [loading, setLoading] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [pilotInfo, setPilotInfo] = useState<HostedPilotBuildInfo | null>(null);
  const [executing, setExecuting] = useState(false);
  // A native invocation outlives a task-selection generation. Prevent another
  // submission until that invocation settles, even if the selected task changes.
  const runInFlight = useRef(false);
  const [pilotResult, setPilotResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // A monotonic generation also rejects ABA edits (A -> B -> A) while an
  // older preview is in flight. A matching text key alone is not sufficient.
  const generation = useRef(0);
  useEffect(() => {
    let active = true;
    setPilotInfo(null);
    void hostedPilotBuildInfo().then((info) => {
      if (active && info.enabled && info.syntheticOnly) setPilotInfo(info);
    });
    return () => { active = false; };
  }, []);


  const invalidateReview = () => {
    generation.current += 1;
    setPreview(null);
    setVerified(null);
    setError(null);
    setLoading(false);
    setVerifying(false);
    setPilotResult(null);
  };

  useEffect(() => {
    generation.current += 1;
    setOpened(false);
    setReviewedTitle("");
    setObjective("");
    setOutputsText("");
    setPreview(null);
    setVerified(null);
    setLoading(false);
    setVerifying(false);
    setPilotResult(null);
    setError(null);
  }, [workspaceId, taskId]);

  const reviewedOutputs = requestedOutputsFromText(outputsText);
  const valid = reviewedTitle.trim().length > 0 && objective.trim().length > 0 && reviewedOutputs.length > 0;

  const showScope = async () => {
    if (!valid || loading || verifying || executing) return;
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
    setVerified(null);
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

  const verifyPreparedBytes = async () => {
    if (!preview || loading || verifying || executing) return;
    const currentGeneration = generation.current;
    const request: PreviewHostedTaskExportRequest = {
      workspaceId,
      taskId,
      reviewedExportBrief: {
        title: reviewedTitle.trim(),
        objective: objective.trim(),
        requestedOutputs: reviewedOutputs,
      },
    };
    setVerifying(true);
    setVerified(null);
    setError(null);
    try {
      const result = await verifyHostedTaskExport(request);
      if (generation.current !== currentGeneration) return;
      if (!result.verifiedBytes || result.exportAuthorized
        || result.bundleMetadataSha256 !== preview.bundleMetadataSha256
        || result.inputFileCount !== preview.inputFileCount
        || result.inputTotalBytes !== preview.inputTotalBytes) {
        throw new Error("Scope mismatch");
      }
      setVerified(result);
    } catch {
      if (generation.current !== currentGeneration) return;
      setError("Actual prepared-byte verification failed or the reviewed scope changed. Recheck the task before proceeding.");
    } finally {
      if (generation.current === currentGeneration) setVerifying(false);
    }
  };

  const runSyntheticPilot = async () => {
    if (!pilotInfo?.enabled || !verified || !preview || runInFlight.current || loading || verifying) return;
    const request: PreviewHostedTaskExportRequest = {
      workspaceId,
      taskId,
      reviewedExportBrief: {
        title: reviewedTitle.trim(),
        objective: objective.trim(),
        requestedOutputs: reviewedOutputs,
      },
    };
    const currentGeneration = generation.current;
    runInFlight.current = true;
    setExecuting(true);
    setError(null);
    setPilotResult(null);
    try {
      const result = await runHostedTaskPilot(request);
      if (generation.current !== currentGeneration) return;
      setPilotResult(`Imported ${result.outputFileCount} verified output file(s), ${formatBytes(result.outputTotalBytes)}. Review the task outputs separately before publishing.`);
      // Results cannot be reused as a second export approval.
      setVerified(null);
      setPreview(null);
    } catch {
      if (generation.current !== currentGeneration) return;
      setError("Hosted synthetic pilot did not complete. The fixture may have been transferred or charged. Inspect private execution history before attempting another run. No automatic retry.");
      setVerified(null);
    } finally {
      // Never let an old task's result update the new task, but always release
      // this component's invocation lock. Selection is not cancellation.
      runInFlight.current = false;
      setExecuting(false);
    }
  };

  return (
    <section className="mt-3 rounded-xl border bg-muted/15 p-4" data-testid="hosted-export-review">
      <button
        type="button"
        className="flex w-full items-center justify-between text-left text-sm font-medium"
        aria-expanded={opened}
        disabled={executing}
        onClick={() => {
          if (executing) return;
          if (opened) invalidateReview();
          setOpened(!opened);
        }}
      >
        <span className="flex items-center gap-2">
          <FileCheck2 className="h-4 w-4 text-primary" />
          Review remote sandbox export
        </span>
        <span className="text-xs text-muted-foreground">{opened ? "Hide" : "Show"}</span>
      </button>
      {executing ? (
        <p aria-live="polite" className="mt-3 text-xs text-muted-foreground">
          A native hosted pilot request is in progress. Changing tasks does not cancel it.
          Inspect private execution history after it finishes.
        </p>
      ) : null}
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
              disabled={executing}
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
              disabled={executing}
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
              disabled={executing}
              placeholder="outputs/summary.md"
            />
          </label>
          <Button variant="outline" size="sm" onClick={() => void showScope()} disabled={!valid || loading || verifying || executing}>
            {loading ? "Checking scope…" : "Preview export scope (no upload)"}
          </Button>
          {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
          {pilotResult ? <p role="status" className="text-sm font-medium">{pilotResult}</p> : null}
          {preview ? (
            <div className="space-y-3 rounded-md border bg-background p-3 text-xs" data-testid="hosted-export-preview">
              <div className="font-medium">
                {preview.inputFileCount} declared input file{preview.inputFileCount === 1 ? "" : "s"}
                {" · "}{formatBytes(preview.inputTotalBytes)} · ${(preview.pilotSpendLimitUsdCents / 100).toFixed(2)} USD requested best-effort limit (not guaranteed)
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
              <Button variant="outline" size="sm" disabled={loading || verifying || executing} onClick={() => void verifyPreparedBytes()}>
                {verifying ? "Verifying prepared files…" : "Verify actual prepared bytes (no upload)"}
              </Button>
              {verified ? (
                <p className="font-medium" data-testid="hosted-export-verified">
                  Actual prepared bytes matched the reviewed manifest at the time of this check.
                  This is not export or spending approval. Any future execution must reverify bytes.
                </p>
              ) : null}
              {pilotInfo?.enabled && verified ? (
                <div className="space-y-2 rounded-md border p-3" data-testid="hosted-synthetic-pilot">
                  <p className="font-medium">
                    Operator-only synthetic pilot. Only Infimount's exact public fixture is permitted.
                    A separate native dialog will show the upload scope and request explicit approval.
                    OpenAI U.S.-only/non-ZDR terms apply and the US$1 best-effort limit is not guaranteed.
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => void runSyntheticPilot()}
                    disabled={executing || loading || verifying}
                  >
                    {executing ? "Running hosted synthetic pilot…" : "Review natively and run synthetic pilot"}
                  </Button>
                </div>
              ) : null}
              <p className="font-medium">
                No data transferred by preview or byte verification. Neither authorizes spending or publication.
                A hosted run is a separate action and may transfer the synthetic fixture after native approval.
              </p>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
