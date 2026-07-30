import { Info, ShieldCheck, Trash2, ToggleLeft, ToggleRight } from "lucide-react";
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
import { clearProductEvents, setTelemetryConsent } from "@/lib/api";

interface PrivacySettingsProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentConsent: "unknown" | "granted" | "denied";
  onConsentChange: (consent: "granted" | "denied") => void;
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
      const enabled = currentConsent !== "granted";
      await setTelemetryConsent({ consent: enabled });
      const newConsent = enabled ? "granted" : "denied";
      onConsentChange(newConsent);
      toast({
        title: enabled ? "Telemetry enabled" : "Telemetry disabled",
        description: enabled
          ? "Schema-limited telemetry can be sent when an operator endpoint is configured."
          : "Network telemetry is off. The bounded local event log is unchanged.",
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
                  Infimount keeps a bounded local product-event log for diagnostics. When
                  enabled, schema-limited product events and MCP operation metrics may be sent
                  only if an operator has configured a valid telemetry endpoint. Storage names,
                  file paths, storage endpoints, credentials, and file contents are excluded.
                </p>
              </div>
              <button
                type="button"
                onClick={handleToggle}
                disabled={isSaving}
                className="shrink-0"
                aria-label={currentConsent === "granted" ? "Disable telemetry" : "Enable telemetry"}
              >
                {currentConsent === "granted" ? (
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
                <strong>Local-first by default.</strong> Network telemetry is disabled until
                you explicitly enable it, and no transmission occurs without a configured
                endpoint. Revoking consent stops sends immediately. The bounded local event log is
                managed separately and can be cleared with the control below.
              </div>
            </div>
          </div>

          <div className="rounded-xl border border-border/80 bg-card p-4">
            <div className="text-sm font-medium">Eligible telemetry fields</div>
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

        <div className="flex justify-between gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={async () => {
              try {
                await clearProductEvents();
                toast({ title: "Local event log cleared" });
              } catch {
                toast({ title: "Failed to clear local event log", variant: "destructive" });
              }
            }}
          >
            <Trash2 className="mr-1 h-4 w-4" />
            Clear local events
          </Button>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
