import { useState, type Dispatch, type SetStateAction } from "react";
import { Play, Square, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { McpRuntimeStatus, McpSettings } from "@/types/storage";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

interface McpRuntimeSectionProps {
  settings: McpSettings;
  onSettingsChange: Dispatch<SetStateAction<McpSettings>>;
  status: McpRuntimeStatus | null;
  authTokenDraft: string | undefined;
  onAuthTokenDraftChange: (value: string | undefined) => void;
  isBusy: boolean;
  isSaving: boolean;
  isTogglingHttp: boolean;
  nonLoopbackMissingAuth: boolean;
  showNetworkWarning: boolean;
  requiresHttpRestart: boolean;
  primaryActionLabel: string;
  endpointDisplay: string;
  onSave: () => void;
  onRotateAuthToken: () => void;
  onHttpToggle: () => void;
}

export function McpRuntimeSection({
  settings,
  onSettingsChange,
  status,
  authTokenDraft,
  onAuthTokenDraftChange,
  isBusy,
  isSaving,
  isTogglingHttp,
  nonLoopbackMissingAuth,
  showNetworkWarning,
  requiresHttpRestart,
  primaryActionLabel,
  endpointDisplay,
  onSave,
  onRotateAuthToken,
  onHttpToggle,
}: McpRuntimeSectionProps) {
  const [dismissedNetworkWarningKey, setDismissedNetworkWarningKey] = useState<string | null>(null);
  const [dismissedRestartWarningKey, setDismissedRestartWarningKey] = useState<string | null>(null);
  const isStdio = settings.transport === "stdio";
  const stdioReady = isStdio && settings.enabled;
  const runtimeLabel = isStdio
    ? stdioReady
      ? "On demand"
      : "Disabled"
    : status?.runningHttp
      ? "Running"
      : "Stopped";
  const runtimeHealthy = isStdio ? stdioReady : Boolean(status?.runningHttp);
  const networkWarningKey = showNetworkWarning ? settings.bindAddress.trim() : null;
  const restartWarningKey = requiresHttpRestart
    ? JSON.stringify([
        settings.bindAddress,
        settings.port,
        settings.enabledTools,
        authTokenDraft ?? null,
      ])
    : null;
  const networkWarningVisible = Boolean(
    networkWarningKey && dismissedNetworkWarningKey !== networkWarningKey,
  );
  const restartWarningVisible = Boolean(
    restartWarningKey && dismissedRestartWarningKey !== restartWarningKey,
  );

  return (
    <div className="grid gap-4 rounded-xl border border-border/80 bg-secondary/35 p-4 md:grid-cols-[1.1fr_0.9fr]">
      <div className="space-y-4">
        <div>
          <Label className="text-sm font-medium text-foreground">Transport Settings</Label>
          <p className="mt-1 text-[11px] text-muted-foreground">
            stdio is client-launched on demand. HTTP runs as a persistent local server.
          </p>
        </div>

        <div className="space-y-2">
          <Label className="text-xs font-normal text-muted-foreground">Transport</Label>
          <Select
            value={settings.transport}
            onValueChange={(value) =>
              onSettingsChange((current) => ({
                ...current,
                transport: value as McpSettings["transport"],
              }))
            }
          >
            <SelectTrigger
              className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
              <SelectItem value="stdio">stdio</SelectItem>
              <SelectItem value="http">http</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {isStdio ? (
          <div className="rounded-lg border border-border/80 bg-card p-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-medium text-foreground">General Agent Access</div>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  When enabled, configured MCP clients may launch the bundled stdio sidecar. When
                  disabled, general stdio connections fail closed before tools are exposed.
                </p>
              </div>
              <Switch
                aria-label="Enable general Agent Access"
                checked={settings.enabled}
                disabled={isBusy}
                onCheckedChange={(enabled) =>
                  onSettingsChange((current) => ({ ...current, enabled }))
                }
              />
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label className="text-xs font-normal text-muted-foreground">
                  Bind Address
                </Label>
                <Input
                  value={settings.bindAddress}
                  onChange={(event) =>
                    onSettingsChange((current) => ({
                      ...current,
                      bindAddress: event.target.value,
                    }))
                  }
                  className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                />
              </div>
              <div className="space-y-2">
                <Label className="text-xs font-normal text-muted-foreground">Port</Label>
                <Input
                  type="number"
                  min={0}
                  max={65535}
                  value={settings.port}
                  onChange={(event) =>
                    onSettingsChange((current) => ({
                      ...current,
                      port: Number.parseInt(event.target.value || "0", 10),
                    }))
                  }
                  className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                />
              </div>
            </div>
            <div className="space-y-2">
              <Label className="text-xs font-normal text-muted-foreground">
                HTTP bearer token
              </Label>
              <Input
                type="password"
                value={authTokenDraft ?? ""}
                placeholder={
                  settings.authTokenConfigured
                    ? "Stored locally — leave blank to keep"
                    : showNetworkWarning
                      ? "Required for non-loopback HTTP"
                      : "Optional for loopback HTTP"
                }
                onChange={(event) => onAuthTokenDraftChange(event.target.value)}
                className={`border border-border bg-card text-sm text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
              />
              <div className="flex items-center justify-between gap-3">
                <p className="text-[11px] text-muted-foreground">
                  {settings.authTokenConfigured
                    ? "A token is stored locally. Enter a replacement, leave untouched to keep it, or clear it explicitly."
                    : "Clients must send Authorization: Bearer … when a token is set."}
                </p>
                {settings.authTokenConfigured ? (
                  <div className="flex shrink-0 gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={isBusy}
                      onClick={onRotateAuthToken}
                    >
                      Rotate token
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={isBusy}
                      onClick={() => onAuthTokenDraftChange("")}
                    >
                      Clear token
                    </Button>
                  </div>
                ) : null}
              </div>
            </div>
            {networkWarningVisible ? (
              <div className="flex items-start gap-2 rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                <span className="min-w-0 flex-1">
                  This bind address is not loopback. Clients on your LAN may be able to reach this MCP
                  endpoint. A bearer token is required before it can start.
                </span>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6 shrink-0"
                  aria-label="Dismiss network exposure warning"
                  title="Dismiss network exposure warning"
                  onClick={() => setDismissedNetworkWarningKey(networkWarningKey)}
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </Button>
              </div>
            ) : null}
          </div>
        )}
      </div>

      <div className="space-y-3 rounded-lg border border-border/80 bg-card p-4 shadow-sm">
        <div className="flex items-center justify-between gap-3">
          <div>
            <div className="text-sm font-medium text-foreground">
              {isStdio ? "Agent Access Status" : "HTTP Runtime Status"}
            </div>
            <p className="mt-1 text-[11px] text-muted-foreground">
              {isStdio
                ? stdioReady
                  ? "Ready for client launch. No background server is required."
                  : "General MCP access is disabled."
                : status?.runningHttp
                  ? "HTTP server is live."
                  : "HTTP server is not running."}
            </p>
          </div>
          <div
            className={`rounded-full px-2.5 py-1 text-[10px] font-medium uppercase tracking-[0.14em] ${
              runtimeHealthy
                ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                : "bg-muted text-muted-foreground"
            }`}
          >
            {runtimeLabel}
          </div>
        </div>

        <div className="rounded-lg border border-border/80 bg-background p-3">
          <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
            {isStdio ? "Launch mode" : "Endpoint"}
          </div>
          <div className="mt-2 break-all font-mono text-xs text-foreground">
            {isStdio
              ? settings.enabled
                ? "Client-launched stdio"
                : "Agent access disabled"
              : endpointDisplay}
          </div>
        </div>

        {isStdio ? (
          <Button
            type="button"
            className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
            onClick={onSave}
            disabled={isBusy}
          >
            {isSaving ? "Saving..." : primaryActionLabel}
          </Button>
        ) : (
          <Button
            type="button"
            className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
            onClick={onHttpToggle}
            disabled={isBusy || (!status?.runningHttp && nonLoopbackMissingAuth)}
          >
            {status?.runningHttp ? (
              <Square className="mr-2 h-4 w-4" />
            ) : (
              <Play className="mr-2 h-4 w-4" />
            )}
            {isTogglingHttp ? "Working..." : primaryActionLabel}
          </Button>
        )}

        {!isStdio && restartWarningVisible ? (
          <div className="flex items-start gap-2 rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
            <span className="min-w-0 flex-1">
              MCP settings changed. Restart the HTTP server (Stop then Start) to apply these changes.
            </span>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="h-6 w-6 shrink-0"
              aria-label="Dismiss restart warning"
              title="Dismiss restart warning"
              onClick={() => setDismissedRestartWarningKey(restartWarningKey)}
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}
