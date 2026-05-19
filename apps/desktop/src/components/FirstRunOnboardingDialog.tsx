import { CheckCircle2, Database, PlugZap, ShieldCheck, TestTube2 } from "lucide-react";
import type { ReactNode } from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface FirstRunOnboardingDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAddStorage: () => void;
  onOpenMcpSettings: () => void;
  onTestConnection: () => Promise<void>;
  onComplete: () => Promise<void>;
  onSkip: () => Promise<void>;
}

export function FirstRunOnboardingDialog({
  open,
  onOpenChange,
  onAddStorage,
  onOpenMcpSettings,
  onTestConnection,
  onComplete,
  onSkip,
}: FirstRunOnboardingDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[720px] rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-xl font-semibold">
            Set up Infimount safely
          </DialogTitle>
          <DialogDescription className="text-left text-sm text-muted-foreground">
            Add storage, expose only what you choose, and connect an MCP client without leaving the
            app.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 md:grid-cols-2">
          <OnboardingStep
            icon={<Database className="h-4 w-4" />}
            title="Add your first storage"
            description="Start with a local folder or any configured OpenDAL backend."
            actionLabel="Add storage"
            onAction={onAddStorage}
          />
          <OnboardingStep
            icon={<ShieldCheck className="h-4 w-4" />}
            title="Enable MCP safely"
            description="Review exposed storages and functions before an agent can use them."
            actionLabel="Open MCP settings"
            onAction={onOpenMcpSettings}
          />
          <OnboardingStep
            icon={<PlugZap className="h-4 w-4" />}
            title="Connect an MCP client"
            description="Copy stdio or HTTP snippets generated from your current runtime settings."
            actionLabel="View snippets"
            onAction={onOpenMcpSettings}
          />
          <OnboardingStep
            icon={<TestTube2 className="h-4 w-4" />}
            title="Test connection"
            description="Check runtime status, enabled functions, and currently exposed storages."
            actionLabel="Run test"
            onAction={() => void onTestConnection()}
          />
        </div>

        <div className="rounded-xl border border-border/80 bg-secondary/35 p-4">
          <div className="flex items-start gap-3">
            <CheckCircle2 className="mt-0.5 h-4 w-4 text-primary" />
            <div className="space-y-1">
              <div className="text-sm font-medium text-foreground">Local-first by default</div>
              <p className="text-xs leading-5 text-muted-foreground">
                Storage credentials and MCP settings stay on this machine under your Infimount
                config directory. Sample or real storage is not exposed to MCP without your consent.
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap justify-between gap-3">
          <Button type="button" variant="ghost" onClick={() => void onSkip()}>
            Skip for now
          </Button>
          <Button type="button" onClick={() => void onComplete()}>
            Finish setup
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function OnboardingStep({
  icon,
  title,
  description,
  actionLabel,
  onAction,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  actionLabel: string;
  onAction: () => void;
}) {
  return (
    <div className="flex h-full flex-col justify-between rounded-xl border border-border/80 bg-card p-4 shadow-sm">
      <div className="space-y-3">
        <div className="flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-background text-primary">
          {icon}
        </div>
        <div>
          <div className="text-sm font-medium text-foreground">{title}</div>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p>
        </div>
      </div>
      <Button
        type="button"
        variant="outline"
        className="mt-4 border-border/80"
        onClick={onAction}
      >
        {actionLabel}
      </Button>
    </div>
  );
}
