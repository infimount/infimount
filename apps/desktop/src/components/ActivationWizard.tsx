import {
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Database,
  PlugZap,
  ShieldCheck,
  Sparkles,
  Terminal,
  TestTube2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  applyMcpClientInstall,
  listMcpClientAdapters,
  previewMcpClientInstall,
  rollbackMcpClientInstall,
  runActivationProbe,
} from "@/lib/api";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import type {
  ActivationProbeOutput,
  McpClientAdapterInfo,
  McpClientInstallPreview,
  McpRuntimeStatus,
} from "@/types/storage";

export type WizardStepId =
  | "welcome"
  | "storage"
  | "workspace"
  | "mcp"
  | "client"
  | "verify"
  | "done";

export interface ActivationWizardProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAddStorage: () => void;
  onCreateDemo: () => Promise<void>;
  onOpenWorkspaces: () => void;
  onOpenMcpSettings: () => void;
  onComplete: () => Promise<void>;
  onSkip: () => Promise<void>;
  onSaveState: (step: WizardStepId | null, completed: WizardStepId[]) => Promise<void>;
  storagesCount: number;
  workspacesCount: number;
  mcpStatus?: McpRuntimeStatus;
  initialStep?: string | null;
  initialCompletedSteps?: string[];
}

const STEP_ORDER: WizardStepId[] = [
  "welcome",
  "storage",
  "workspace",
  "mcp",
  "client",
  "verify",
  "done",
];

const STEP_LABELS: Record<WizardStepId, string> = {
  welcome: "Welcome",
  storage: "Add Storage",
  workspace: "Scope Workspace",
  mcp: "Verify Sidecar",
  client: "Connect Client",
  verify: "Verify",
  done: "Done",
};

const STEP_ICONS: Record<WizardStepId, typeof Sparkles> = {
  welcome: Sparkles,
  storage: Database,
  workspace: ShieldCheck,
  mcp: Terminal,
  client: PlugZap,
  verify: TestTube2,
  done: CheckCircle2,
};

export function ActivationWizard({
  open,
  onOpenChange,
  onAddStorage,
  onCreateDemo,
  onOpenWorkspaces,
  onOpenMcpSettings,
  onComplete,
  onSkip,
  onSaveState,
  storagesCount,
  workspacesCount,
  mcpStatus,
  initialStep,
  initialCompletedSteps = [],
}: ActivationWizardProps) {
  const validInitialStep = initialStep && STEP_ORDER.includes(initialStep as WizardStepId)
    ? initialStep as WizardStepId
    : "welcome";
  const validInitialCompletedSteps = initialCompletedSteps.filter(
    (step): step is WizardStepId => STEP_ORDER.includes(step as WizardStepId),
  );
  const [currentStep, setCurrentStep] = useState<WizardStepId>(validInitialStep);
  const [completedSteps, setCompletedSteps] = useState<WizardStepId[]>(validInitialCompletedSteps);
  const [probe, setProbe] = useState<ActivationProbeOutput>();
  const [probeRunning, setProbeRunning] = useState(false);
  const [probeRequestError, setProbeRequestError] = useState(false);
  const [demoCreating, setDemoCreating] = useState(false);
  const [demoError, setDemoError] = useState(false);
  const [clientReviewed, setClientReviewed] = useState(false);
  const [finishRunning, setFinishRunning] = useState(false);
  const [finishError, setFinishError] = useState<string>();

  const currentIndex = STEP_ORDER.indexOf(currentStep);

  useEffect(() => {
    if (!open) return;
    setCurrentStep(
      initialStep && STEP_ORDER.includes(initialStep as WizardStepId)
        ? initialStep as WizardStepId
        : "welcome",
    );
    setCompletedSteps(initialCompletedSteps.filter(
      (step): step is WizardStepId => STEP_ORDER.includes(step as WizardStepId),
    ));
    setClientReviewed(false);
    setFinishError(undefined);
  }, [initialCompletedSteps, initialStep, open]);

  const isStepComplete = (step: WizardStepId) => completedSteps.includes(step);
  const isStepCurrent = (step: WizardStepId) => step === currentStep;

  const goToStep = (step: WizardStepId) => {
    const targetIndex = STEP_ORDER.indexOf(step);
    const furthestUnlocked = Math.min(
      STEP_ORDER.length - 1,
      completedSteps.reduce(
        (furthest, completed) => Math.max(furthest, STEP_ORDER.indexOf(completed) + 1),
        0,
      ),
    );
    if (targetIndex > furthestUnlocked) return;
    setCurrentStep(step);
    void onSaveState(step, completedSteps);
  };

  const completeStep = (step: WizardStepId) => {
    const next = completedSteps.includes(step)
      ? completedSteps
      : [...completedSteps, step];
    setCompletedSteps(next);
    return next;
  };

  const goNext = () => {
    const nextCompleted = completeStep(currentStep);
    const nextIndex = currentIndex + 1;
    if (nextIndex < STEP_ORDER.length) {
      const nextStep = STEP_ORDER[nextIndex];
      setCurrentStep(nextStep);
      void onSaveState(nextStep, nextCompleted);
    }
  };

  const goBack = () => {
    const prevIndex = currentIndex - 1;
    if (prevIndex >= 0) {
      const previousStep = STEP_ORDER[prevIndex];
      setCurrentStep(previousStep);
      void onSaveState(previousStep, completedSteps);
    }
  };

  const canGoNext = useMemo(() => {
    switch (currentStep) {
      case "welcome": return true;
      case "storage": return storagesCount > 0;
      case "workspace": return workspacesCount > 0;
      case "mcp": return mcpStatus?.settings.enabled === true
        && probe?.sidecar.versionMatch === true
        && probe.sidecar.doctorHealthy === true;
      case "client": return clientReviewed;
      case "verify": return probe?.overallOk === true;
      case "done": return false;
    }
  }, [
    clientReviewed,
    currentStep,
    mcpStatus?.settings.enabled,
    probe?.overallOk,
    probe?.sidecar.doctorHealthy,
    probe?.sidecar.versionMatch,
    storagesCount,
    workspacesCount,
  ]);

  const handleRunProbe = async () => {
    setProbeRunning(true);
    setProbeRequestError(false);
    try {
      const result = await runActivationProbe();
      setProbe(result);
    } catch {
      setProbe(undefined);
      setProbeRequestError(true);
    } finally {
      setProbeRunning(false);
    }
  };

  const handleFinish = async () => {
    if (!probe?.overallOk || finishRunning) return;
    setFinishRunning(true);
    setFinishError(undefined);
    try {
      await onComplete();
      completeStep("done");
    } catch (error) {
      setFinishError(error instanceof Error ? error.message : String(error));
    } finally {
      setFinishRunning(false);
    }
  };

  const StepIcon = STEP_ICONS[currentStep];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="sm:max-w-[800px] rounded-2xl border border-border bg-background text-foreground shadow-2xl"
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-xl font-semibold">
            <StepIcon className="h-5 w-5 text-primary" />
            {STEP_LABELS[currentStep]}
          </DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            Step {currentIndex + 1} of {STEP_ORDER.length}
          </DialogDescription>
        </DialogHeader>

        {/* Progress bar */}
        <div className="flex gap-1.5">
          {STEP_ORDER.map((step) => (
            <button
              key={step}
              type="button"
              onClick={() => goToStep(step)}
              disabled={
                STEP_ORDER.indexOf(step) >
                completedSteps.reduce(
                  (furthest, completed) =>
                    Math.max(furthest, STEP_ORDER.indexOf(completed) + 1),
                  0,
                )
              }
              className={`flex h-2 flex-1 rounded-full transition-colors ${
                isStepComplete(step) || isStepCurrent(step)
                  ? "bg-primary"
                  : "bg-muted"
              }`}
              aria-label={`Go to step ${STEP_LABELS[step]}`}
            />
          ))}
        </div>

        <Separator />

        {/* Step content */}
        <div className="min-h-[280px]">
          {currentStep === "welcome" && <WelcomeStep />}
          {currentStep === "storage" && (
            <StorageStep
              storagesCount={storagesCount}
              onAddStorage={onAddStorage}
              onCreateDemo={async () => {
                setDemoCreating(true);
                setDemoError(false);
                try {
                  await onCreateDemo();
                } catch {
                  setDemoError(true);
                } finally {
                  setDemoCreating(false);
                }
              }}
              demoCreating={demoCreating}
              demoError={demoError}
            />
          )}
          {currentStep === "workspace" && (
            <WorkspaceStep
              workspacesCount={workspacesCount}
              onOpenWorkspaces={onOpenWorkspaces}
            />
          )}
          {currentStep === "mcp" && (
            <McpStep
              mcpStatus={mcpStatus}
              sidecar={probe?.sidecar}
              probeRunning={probeRunning}
              onValidateSidecar={handleRunProbe}
              onOpenMcpSettings={onOpenMcpSettings}
            />
          )}
          {currentStep === "client" && <ClientStep onReviewed={() => setClientReviewed(true)} />}
          {currentStep === "verify" && (
            <VerifyStep
              probe={probe}
              running={probeRunning}
              requestError={probeRequestError}
              onRun={handleRunProbe}
            />
          )}
          {currentStep === "done" && <DoneStep />}
        </div>

        <Separator />

        {finishError ? (
          <p role="alert" className="text-sm text-destructive">
            Final server-side verification failed: {finishError}
          </p>
        ) : null}

        {/* Navigation */}
        <div className="flex items-center justify-between">
          <div className="flex gap-2">
            {currentIndex > 0 && (
              <Button type="button" variant="ghost" onClick={goBack}>
                <ChevronLeft className="mr-1 h-4 w-4" />
                Back
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            <Button type="button" variant="ghost" onClick={() => void onSkip()}>
              Skip
            </Button>
            {currentStep === "done" ? (
              <Button
                type="button"
                onClick={() => void handleFinish()}
                disabled={!probe?.overallOk || finishRunning}
              >
                {finishRunning ? "Verifying again…" : "Finish"}
              </Button>
            ) : (
              <Button type="button" onClick={goNext} disabled={!canGoNext}>
                Continue
                <ChevronRight className="ml-1 h-4 w-4" />
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function WelcomeStep() {
  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <Sparkles className="h-4 w-4 text-primary" />
          Welcome to Infimount
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Infimount starts with read-only MCP tools, no administration tools, no whole-storage
          grants, and no exposed storage. Access is added only through an explicit path-scoped
          workspace.
        </p>
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <SafetyCard
          icon={<Database className="h-4 w-4" />}
          title="Browse storage"
          description="Connect local folders, S3, WebDAV, and 15+ backends via OpenDAL."
        />
        <SafetyCard
          icon={<ShieldCheck className="h-4 w-4" />}
          title="Read-only workspace"
          description="Default access is scoped to one workspace path; everything else remains denied."
        />
        <SafetyCard
          icon={<PlugZap className="h-4 w-4" />}
          title="Connect agents"
          description="Claude Code, Cursor, VS Code, OpenCode — any MCP client, stdio or HTTP."
        />
      </div>

      <div className="rounded-xl border border-amber-500/20 bg-amber-50/50 p-3 dark:bg-amber-950/10">
        <p className="text-xs leading-5 text-amber-700 dark:text-amber-400">
          <strong>Local-first.</strong> Credentials and settings stay on your machine under{" "}
          <code className="rounded bg-amber-100 px-1 dark:bg-amber-900/30">
            ~/.infimount/
          </code>
          . Optional product telemetry remains off unless you explicitly opt in later.
        </p>
      </div>
    </div>
  );
}

function StorageStep({
  storagesCount,
  onAddStorage,
  onCreateDemo,
  demoCreating,
  demoError,
}: {
  storagesCount: number;
  onAddStorage: () => void;
  onCreateDemo: () => Promise<void>;
  demoCreating: boolean;
  demoError: boolean;
}) {
  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <Database className="h-4 w-4 text-primary" />
          Add your first storage
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Start by connecting a local folder or a cloud storage backend. You can always add more
          later.
        </p>
      </div>

      {storagesCount > 0 ? (
        <div className="flex items-center gap-2 rounded-xl border border-green-200 bg-green-50 p-3 dark:border-green-900/30 dark:bg-green-950/10">
          <CheckCircle2 className="h-4 w-4 text-green-600" />
          <span className="text-sm text-green-700 dark:text-green-400">
            {storagesCount} storage configuration(s) ready.
          </span>
        </div>
      ) : (
        <Card className="flex flex-col items-center gap-3 p-6">
          <Database className="h-8 w-8 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">
            No storages configured yet.
          </p>
          <div className="flex flex-wrap justify-center gap-2">
            <Button type="button" onClick={() => void onCreateDemo()} disabled={demoCreating}>
              {demoCreating ? "Creating safe demo…" : "Create safe demo"}
            </Button>
            <Button type="button" onClick={onAddStorage} variant="outline">
              Add and validate storage
            </Button>
          </div>
          {demoError ? (
            <p role="alert" className="text-xs text-destructive">
              Demo setup failed. No MCP access was broadened; retry or add a storage manually.
            </p>
          ) : null}
        </Card>
      )}
    </div>
  );
}

function WorkspaceStep({
  workspacesCount,
  onOpenWorkspaces,
}: {
  workspacesCount: number;
  onOpenWorkspaces: () => void;
}) {
  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <ShieldCheck className="h-4 w-4 text-primary" />
          Scope access to a workspace
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Create a read-only workspace. Infimount grants only that path; the rest of the storage
          remains denied. The safe demo creates <code>workspace/</code> and keeps
          <code>outside/denied.txt</code> inaccessible.
        </p>
      </div>
      {workspacesCount > 0 ? (
        <div className="flex items-center gap-2 rounded-xl border border-green-200 bg-green-50 p-3 dark:border-green-900/30 dark:bg-green-950/10">
          <CheckCircle2 className="h-4 w-4 text-green-600" />
          <span className="text-sm text-green-700 dark:text-green-400">
            {workspacesCount} scoped workspace{workspacesCount === 1 ? "" : "s"} ready.
          </span>
        </div>
      ) : (
        <Button type="button" onClick={onOpenWorkspaces}>
          Create read-only workspace
        </Button>
      )}
    </div>
  );
}

function McpStep({
  mcpStatus,
  sidecar,
  probeRunning,
  onValidateSidecar,
  onOpenMcpSettings,
}: {
  mcpStatus?: McpRuntimeStatus;
  sidecar?: ActivationProbeOutput["sidecar"];
  probeRunning: boolean;
  onValidateSidecar: () => Promise<void>;
  onOpenMcpSettings: () => void;
}) {
  const isReady = mcpStatus?.runningHttp || mcpStatus?.settings.enabled;
  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <ShieldCheck className="h-4 w-4 text-primary" />
          Configure MCP safely
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          MCP (Model Context Protocol) lets AI agents access your storage. Review which storages
          are exposed, which tools are enabled, and set path-scoped access policies.
        </p>
      </div>

      {mcpStatus ? (
        <div className="rounded-xl border border-border/80 bg-card p-3">
          <div className="space-y-2 text-sm">
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Status</span>
              <span className={isReady ? "text-green-600" : "text-muted-foreground"}>
                {isReady ? "Active" : "Inactive"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Transport</span>
              <span>{mcpStatus.settings.transport}</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Endpoint</span>
              <span className="font-mono text-xs">{mcpStatus.endpointDisplay}</span>
            </div>
          </div>
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" onClick={onOpenMcpSettings}>
          Open MCP settings
        </Button>
        <Button type="button" onClick={() => void onValidateSidecar()} disabled={probeRunning}>
          {probeRunning ? "Validating bundled sidecar…" : "Validate sidecar and policy"}
        </Button>
      </div>
      {sidecar ? (
        <p className={sidecar.versionMatch && sidecar.doctorHealthy ? "text-sm text-green-600" : "text-sm text-destructive"}>
          {sidecar.versionMatch && sidecar.doctorHealthy
            ? `Bundled sidecar ${sidecar.version ?? ""} passed version and doctor checks.`
            : `Sidecar validation failed (${sidecar.errorCode ?? "ERR_SIDECAR_VALIDATION_FAILED"}).`}
        </p>
      ) : null}
    </div>
  );
}

function ClientStep({ onReviewed }: { onReviewed: () => void }) {
  const [adapters, setAdapters] = useState<McpClientAdapterInfo[]>([]);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    void listMcpClientAdapters()
      .then((items) => {
        if (!cancelled) setAdapters(items);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => { cancelled = true; };
  }, []);

  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <PlugZap className="h-4 w-4 text-primary" />
          Connect an MCP client
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Every adapter uses the verified, same-version bundled sidecar. Review exact changes before any config write or command execution.
        </p>
      </div>

      {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
      {!error && adapters.length === 0 ? (
        <p className="text-sm text-muted-foreground">Detecting MCP clients…</p>
      ) : null}
      <div className="grid gap-3 md:grid-cols-2">
        {adapters.map((adapter) => (
          <ClientAdapterCard key={adapter.kind} adapter={adapter} onReviewed={onReviewed} />
        ))}
      </div>
    </div>
  );
}

function ClientAdapterCard({
  adapter,
  onReviewed,
}: {
  adapter: McpClientAdapterInfo;
  onReviewed: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const [targetPath, setTargetPath] = useState(adapter.defaultTarget ?? "");
  const [preview, setPreview] = useState<McpClientInstallPreview>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirmed, setConfirmed] = useState(false);
  const [rollbackId, setRollbackId] = useState<string>();

  const handleCopy = () => {
    void navigator.clipboard.writeText(adapter.snippet).then(() => {
      setCopied(true);
      onReviewed();
      setTimeout(() => setCopied(false), 2000);
    });
  };

  const handlePreview = async () => {
    setBusy(true);
    setError(undefined);
    setRollbackId(undefined);
    try {
      setPreview(await previewMcpClientInstall(adapter.kind, targetPath || undefined));
    } catch (reason: unknown) {
      setPreview(undefined);
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleApply = async () => {
    if (!preview) return;
    setBusy(true);
    setError(undefined);
    try {
      const result = await applyMcpClientInstall(preview.previewId, confirmed);
      setRollbackId(result.rollbackId ?? undefined);
      onReviewed();
      setPreview(undefined);
      setConfirmed(false);
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleRollback = async () => {
    if (!rollbackId) return;
    setBusy(true);
    setError(undefined);
    try {
      await rollbackMcpClientInstall(rollbackId);
      setRollbackId(undefined);
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-xl border border-border/80 bg-card p-3" data-testid={`client-adapter-${adapter.kind}`}>
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex h-7 w-7 items-center justify-center rounded-lg border border-border bg-background text-primary">
          <Terminal className="h-4 w-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div className="text-sm font-medium">{adapter.name}</div>
            <span className="text-[10px] text-muted-foreground">{adapter.detected ? "Detected" : "Not detected"}</span>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">{adapter.description}</p>
          <p className="mt-1 break-all text-[10px] text-muted-foreground">{adapter.detection}</p>

          {adapter.writeCapable && adapter.defaultTarget !== null ? (
            <label className="mt-2 block text-[11px]">
              Config path
              <input
                aria-label={`${adapter.name} config path`}
                className="mt-1 w-full rounded border border-border bg-background px-2 py-1 font-mono text-[10px]"
                value={targetPath}
                onChange={(event) => setTargetPath(event.target.value)}
              />
            </label>
          ) : null}

          <div className="mt-2 max-h-28 overflow-auto rounded-lg bg-muted p-2">
            <pre className="whitespace-pre-wrap break-all font-mono text-[10px] leading-relaxed">{adapter.snippet}</pre>
          </div>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button type="button" variant="outline" size="sm" onClick={handleCopy}>
              {copied ? "Copied!" : "Copy"}
            </Button>
            {adapter.writeCapable ? (
              <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void handlePreview()}>
                Preview install
              </Button>
            ) : null}
            {rollbackId ? (
              <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void handleRollback()}>
                Roll back
              </Button>
            ) : null}
          </div>

          {preview ? (
            <div className="mt-2 space-y-2 rounded border border-border p-2 text-[11px]">
              <div className="font-medium">Reviewed {preview.action} preview (secrets redacted)</div>
              {preview.targetPath ? <div className="break-all">Target: {preview.targetPath}</div> : null}
              {preview.before !== null ? (
                <details><summary>Before</summary><pre className="max-h-24 overflow-auto whitespace-pre-wrap break-all">{preview.before}</pre></details>
              ) : null}
              <details open><summary>After</summary><pre className="max-h-24 overflow-auto whitespace-pre-wrap break-all">{preview.after}</pre></details>
              {preview.requiresExecutionConfirmation ? (
                <label className="flex items-center gap-2">
                  <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} />
                  I confirm execution of this exact command
                </label>
              ) : null}
              <Button
                type="button"
                size="sm"
                disabled={busy || !preview.canApply || (preview.requiresExecutionConfirmation && !confirmed)}
                onClick={() => void handleApply()}
              >
                {preview.requiresExecutionConfirmation ? "Confirm and execute" : "Apply exact change"}
              </Button>
            </div>
          ) : null}
          {error ? <p role="alert" className="mt-2 text-xs text-destructive">{error}</p> : null}
        </div>
      </div>
    </div>
  );
}

function VerifyStep({
  probe,
  running,
  requestError,
  onRun,
}: {
  probe?: ActivationProbeOutput;
  running: boolean;
  requestError: boolean;
  onRun: () => Promise<void>;
}) {
  const checks = [
    { label: "Bundled sidecar verified", ok: probe?.sidecar.versionMatch === true },
    { label: "MCP handshake completed", ok: probe?.mcpHandshakeOk === true },
    { label: "Scope isolation proven", ok: probe?.scopeIsolationPassed === true },
    { label: "Safe default profile active", ok: probe?.safeDefaultProfilePassed === true },
    { label: "Workspace access allowed", ok: probe?.mcpAllowedOpOk === true },
    { label: "Outside-workspace access denied", ok: probe?.mcpDenialProven === true },
    { label: "Allowed and denied operations audited", ok: probe?.mcpAuditOk === true },
  ];

  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <TestTube2 className="h-4 w-4 text-primary" />
          Verify your setup
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          {probe?.overallOk
            ? "The packaged MCP sidecar passed the workspace access and policy-denial checks."
            : "Run the safety probe. Setup cannot finish until allowed workspace access succeeds and outside access is denied."}
        </p>
        {(requestError || probe?.errorCode) && (
          <p className="mt-2 text-xs text-destructive" role="alert">
            Verification failed{probe?.errorCode ? ` (${probe.errorCode})` : ""}. Review MCP and workspace settings, then retry.
          </p>
        )}
        <Button
          type="button"
          variant="outline"
          className="mt-3"
          disabled={running}
          onClick={() => void onRun()}
        >
          {running ? "Running safety probe…" : "Run safety probe"}
        </Button>
      </div>

      <div className="space-y-2">
        {checks.map((check) => (
          <div
            key={check.label}
            className="flex items-center gap-3 rounded-lg border border-border/60 p-3"
          >
            {check.ok ? (
              <CheckCircle2 className="h-4 w-4 shrink-0 text-green-600" />
            ) : (
              <div className="h-4 w-4 shrink-0 rounded-full border-2 border-muted-foreground/30" />
            )}
            <span className="text-sm">{check.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function DoneStep() {
  return (
    <div className="flex flex-col items-center gap-4 py-6">
      <div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary/10">
        <CheckCircle2 className="h-8 w-8 text-primary" />
      </div>
      <div className="text-center">
        <h3 className="text-lg font-semibold">Setup complete!</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Your Infimount environment is ready. You can always revisit settings from the sidebar.
        </p>
      </div>
    </div>
  );
}

function SafetyCard({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="rounded-xl border border-border/80 bg-card p-3">
      <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-border bg-background text-primary">
        {icon}
      </div>
      <div className="mt-3 text-sm font-medium">{title}</div>
      <p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p>
    </div>
  );
}
