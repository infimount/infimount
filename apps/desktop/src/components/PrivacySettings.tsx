import { Info, ShieldCheck, ToggleLeft, ToggleRight } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import { toast } from "@/hooks/use-toast";
import { setTelemetryConsent } from "@/lib/api";

interface PrivacySettingsProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentConsent: boolean | null;
  onConsentChange: (consent: boolean) => void;
}

export function PrivacySettings({
  open,
  onOpenChange,
  currentConsent,
  onConsentChange,
}: PrivacySettingsProps) {
  const [isSaving, setIsSaving] = useState(false);

  const handleToggle = async () => {
    setIsSaving(true);
    try {
      const newConsent = currentConsent !== true;
      await setTelemetryConsent({ consent: newConsent });
      onConsentChange(newConsent);
      toast({
        title: newConsent ? "Telemetry enabled" : "Telemetry disabled",
        description: newConsent
          ? "Anonymous product events may be sent to improve Infimount."
          : "No product events will be sent.",
      });
    } catch {
      toast({
        title: "Failed to update preference",
        variant: "destructive",
      });
    }
    setIsSaving(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" />
            Privacy settings
          </DialogTitle>
          <DialogDescription>
            Control how Infimount handles product telemetry and diagnostics.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="rounded-xl border border-border/80 bg-card p-4">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="text-sm font-medium">Product telemetry</div>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  When enabled, anonymous product events (app launches, storage additions,
                  activation steps) are collected locally and may be sent to help improve
                  Infimount. No storage names, file paths, endpoints, or credentials are
                  included.
                </p>
              </div>
              <button
                type="button"
                onClick={handleToggle}
                disabled={isSaving}
                className="shrink-0"
                aria-label={currentConsent ? "Disable telemetry" : "Enable telemetry"}
              >
                {currentConsent ? (
                  <ToggleRight className="h-8 w-8 text-primary" />
                ) : (
                  <ToggleLeft className="h-8 w-8 text-muted-foreground" />
                )}
              </button>
            </div>
          </div>

          <div className="rounded-xl border border-amber-500/20 bg-amber-50/50 p-3 dark:bg-amber-950/10">
            <div className="flex items-start gap-2">
              <Info className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
              <div className="text-xs leading-5 text-amber-700 dark:text-amber-400">
                <strong>Local-first by default.</strong> Telemetry is disabled until you
                explicitly enable it. You can revoke consent at any time. The local event
                log can be cleared separately.
              </div>
            </div>
          </div>

          <div className="rounded-xl border border-border/80 bg-card p-4">
            <div className="text-sm font-medium">Data collected</div>
            <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
              <li className="flex items-center gap-2">
                <span className="h-1 w-1 rounded-full bg-muted-foreground/50" />
                App version, OS, and architecture
              </li>
              <li className="flex items-center gap-2">
                <span className="h-1 w-1 rounded-full bg-muted-foreground/50" />
                Anonymous event names (app_launched, storage_added, etc.)
              </li>
              <li className="flex items-center gap-2">
                <span className="h-1 w-1 rounded-full bg-muted-foreground/50" />
                Duration buckets and success/failure flags
              </li>
            </ul>
          </div>
        </div>

        <Separator />

        <div className="flex justify-end">
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
