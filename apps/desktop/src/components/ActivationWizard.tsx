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
import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import type { McpClientSnippets, McpRuntimeStatus } from "@/types/storage";

export type WizardStepId = "welcome" | "storage" | "mcp" | "client" | "verify" | "done";

export interface ActivationWizardProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAddStorage: () => void;
  onOpenMcpSettings: () => void;
  onComplete: () => Promise<void>;
  onSkip: () => Promise<void>;
  onSaveState: (step: WizardStepId | null, completed: WizardStepId[]) => Promise<void>;
  storagesCount: number;
  mcpStatus?: McpRuntimeStatus;
  clientSnippets?: McpClientSnippets;
}

const STEP_ORDER: WizardStepId[] = ["welcome", "storage", "mcp", "client", "verify", "done"];

const STEP_LABELS: Record<WizardStepId, string> = {
  welcome: "Welcome",
  storage: "Add Storage",
  mcp: "MCP Safety",
  client: "Connect Client",
  verify: "Verify",
  done: "Done",
};

const STEP_ICONS: Record<WizardStepId, typeof Sparkles> = {
  welcome: Sparkles,
  storage: Database,
  mcp: ShieldCheck,
  client: PlugZap,
  verify: TestTube2,
  done: CheckCircle2,
};

export function ActivationWizard({
  open,
  onOpenChange,
  onAddStorage,
  onOpenMcpSettings,
  onComplete,
  onSkip,
  onSaveState,
  storagesCount,
  mcpStatus,
  clientSnippets,
}: ActivationWizardProps) {
  const [currentStep, setCurrentStep] = useState<WizardStepId>("welcome");
  const [completedSteps, setCompletedSteps] = useState<WizardStepId[]>([]);

  const currentIndex = STEP_ORDER.indexOf(currentStep);

  const isStepComplete = (step: WizardStepId) => completedSteps.includes(step);
  const isStepCurrent = (step: WizardStepId) => step === currentStep;

  const goToStep = (step: WizardStepId) => {
    setCurrentStep(step);
    void onSaveState(step, completedSteps);
  };

  const completeStep = (step: WizardStepId) => {
    const next = completedSteps.includes(step)
      ? completedSteps
      : [...completedSteps, step];
    setCompletedSteps(next);
    void onSaveState(step, next);
  };

  const goNext = () => {
    completeStep(currentStep);
    const nextIndex = currentIndex + 1;
    if (nextIndex < STEP_ORDER.length) {
      setCurrentStep(STEP_ORDER[nextIndex]);
    }
  };

  const goBack = () => {
    const prevIndex = currentIndex - 1;
    if (prevIndex >= 0) {
      setCurrentStep(STEP_ORDER[prevIndex]);
    }
  };

  const canGoNext = useMemo(() => {
    switch (currentStep) {
      case "welcome": return true;
      case "storage": return storagesCount > 0;
      case "mcp": return true;
      case "client": return true;
      case "verify": return true;
      case "done": return false;
    }
  }, [currentStep, storagesCount]);

  const handleFinish = () => {
    completeStep("done");
    void onComplete();
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
            <StorageStep storagesCount={storagesCount} onAddStorage={onAddStorage} />
          )}
          {currentStep === "mcp" && (
            <McpStep
              mcpStatus={mcpStatus}
              onOpenMcpSettings={onOpenMcpSettings}
            />
          )}
          {currentStep === "client" && <ClientStep clientSnippets={clientSnippets} />}
          {currentStep === "verify" && <VerifyStep mcpStatus={mcpStatus} />}
          {currentStep === "done" && <DoneStep />}
        </div>

        <Separator />

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
              <Button type="button" onClick={handleFinish}>
                Finish
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
          Infimount is a local-first storage browser that lets you connect cloud and local storage
          backends, then safely expose them to AI coding agents via the Model Context Protocol (MCP).
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
          title="Control access"
          description="Choose which storages and tools MCP agents can use. Path-scoped policies."
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
          . Nothing is sent to a cloud service.
        </p>
      </div>
    </div>
  );
}

function StorageStep({
  storagesCount,
  onAddStorage,
}: {
  storagesCount: number;
  onAddStorage: () => void;
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
          <Button type="button" onClick={onAddStorage} variant="default">
            Add storage
          </Button>
        </Card>
      )}
    </div>
  );
}

function McpStep({
  mcpStatus,
  onOpenMcpSettings,
}: {
  mcpStatus?: McpRuntimeStatus;
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

      <Button type="button" variant="outline" onClick={onOpenMcpSettings}>
        Open MCP settings
      </Button>
    </div>
  );
}

function ClientStep({
  clientSnippets,
}: {
  clientSnippets?: McpClientSnippets;
}) {
  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <PlugZap className="h-4 w-4 text-primary" />
          Connect an MCP client
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Add this configuration to your MCP client to give it access to your exposed storages.
        </p>
      </div>

      <div className="grid gap-3">
        <ClientAdapterCard
          name="Claude Code / Cursor / VS Code"
          description="Add the stdio configuration to your MCP client settings file."
          icon={<Terminal className="h-4 w-4" />}
          snippet={clientSnippets?.stdio ?? null}
          label="Stdio config"
        />
        <ClientAdapterCard
          name="OpenCode / Claude Desktop"
          description="Use the HTTP endpoint for network-connected clients."
          icon={<Terminal className="h-4 w-4" />}
          snippet={clientSnippets?.http ?? null}
          label="HTTP config"
        />
      </div>
    </div>
  );
}

function ClientAdapterCard({
  name,
  description,
  icon,
  snippet,
  label,
}: {
  name: string;
  description: string;
  icon: React.ReactNode;
  snippet: string | null;
  label: string;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    if (!snippet) return;
    void navigator.clipboard.writeText(snippet);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="rounded-xl border border-border/80 bg-card p-3">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex h-7 w-7 items-center justify-center rounded-lg border border-border bg-background text-primary">
          {icon}
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium">{name}</div>
          <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
          {snippet ? (
            <div className="mt-2">
              <div className="max-h-32 overflow-auto rounded-lg bg-muted p-2">
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed">
                  {snippet}
                </pre>
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="mt-2"
                onClick={handleCopy}
              >
                {copied ? "Copied!" : `Copy ${label}`}
              </Button>
            </div>
          ) : (
            <p className="mt-2 text-xs text-muted-foreground">
              Configure MCP settings to generate a snippet.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function VerifyStep({ mcpStatus }: { mcpStatus?: McpRuntimeStatus }) {
  const checks = [
    { label: "MCP runtime", ok: Boolean(mcpStatus?.runningHttp || mcpStatus?.settings.enabled) },
    { label: "Storage connected", ok: Boolean(mcpStatus) },
    { label: "Tools available", ok: Boolean(mcpStatus?.settings.enabledTools.length) },
  ];

  const allPass = checks.every((c) => c.ok);

  return (
    <div className="space-y-4 py-2">
      <div className="rounded-xl border border-border/80 bg-card p-4">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <TestTube2 className="h-4 w-4 text-primary" />
          Verify your setup
        </h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          {allPass
            ? "Everything looks good. Your Infimount setup is ready for MCP clients."
            : "Some checks did not pass. Review the steps above to complete the setup."}
        </p>
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
