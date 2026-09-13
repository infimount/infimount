import { useState } from "react";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";

interface ActivationReminderProps {
  onFinishSetup: () => void;
}

export function ActivationReminder({ onFinishSetup }: ActivationReminderProps) {
  const [dismissed, setDismissed] = useState(false);

  if (dismissed) return null;

  return (
    <div
      role="status"
      className="fixed bottom-4 left-1/2 z-50 flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2 rounded-xl border border-amber-500/30 bg-background px-3 py-2 shadow-lg"
    >
      <p className="text-sm text-muted-foreground">
        Activation is incomplete. Agent access remains unverified.
      </p>
      <Button
        type="button"
        size="sm"
        className="h-7 shrink-0 rounded-full px-3 text-xs"
        onClick={onFinishSetup}
      >
        Finish setup
      </Button>
      <Button
        type="button"
        size="icon"
        variant="ghost"
        className="h-7 w-7 shrink-0 rounded-full"
        aria-label="Dismiss activation reminder"
        title="Dismiss activation reminder"
        onClick={() => setDismissed(true)}
      >
        <X className="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
    </div>
  );
}
