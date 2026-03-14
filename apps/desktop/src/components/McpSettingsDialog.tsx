import { useEffect, useState } from "react";
import { Copy, Play, Square } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpToolDefinition,
} from "@/types/storage";
import { toast } from "@/hooks/use-toast";

interface McpSettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: McpRuntimeStatus | null;
  snippets: McpClientSnippets | null;
  tools: McpToolDefinition[];
  onSave: (settings: McpSettings) => Promise<void>;
  onStartHttp: () => Promise<void>;
  onStopHttp: () => Promise<void>;
}

export function McpSettingsDialog({
  open,
  onOpenChange,
  status,
  snippets,
  tools,
  onSave,
  onStartHttp,
  onStopHttp,
}: McpSettingsDialogProps) {
  const [settings, setSettings] = useState<McpSettings>({
    enabled: false,
    transport: "stdio",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: [],
  });
  const [isSaving, setIsSaving] = useState(false);
  const [isTogglingHttp, setIsTogglingHttp] = useState(false);
  const isBusy = isSaving || isTogglingHttp;
  const showNetworkWarning =
    settings.transport === "http" && !isLoopbackBindAddress(settings.bindAddress);
  const requiresHttpRestart = Boolean(
    status?.runningHttp
    && settings.transport === "http"
    && (
      settings.bindAddress !== status.settings.bindAddress
      || settings.port !== status.settings.port
      || !sameToolSet(settings.enabledTools, status.settings.enabledTools)
    ),
  );

  useEffect(() => {
    if (!status || !open) return;
    setSettings(status.settings);
  }, [open, status]);

  const handleCopy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    toast({
      title: "Copied",
      description: "Snippet copied to clipboard.",
    });
  };

  const handleSave = async () => {
    setIsSaving(true);
    try {
      await onSave({ ...settings, enabled: false });
    } finally {
      setIsSaving(false);
    }
  };

  const handleHttpToggle = async () => {
    if (showNetworkWarning) {
      const confirmed = window.confirm(
        `The MCP HTTP server will start on ${settings.bindAddress}, which may expose it beyond this machine.\n\nDo you want to continue?`,
      );
      if (!confirmed) return;
    }

    setIsTogglingHttp(true);
    try {
      if (status?.runningHttp) {
        await onSave({ ...settings, enabled: false });
        await onStopHttp();
      } else {
        await onSave({ ...settings, enabled: true });
        await onStartHttp();
      }
    } finally {
      setIsTogglingHttp(false);
    }
  };

  const endpointDisplay = status?.endpointDisplay ?? "Not configured yet";
  const enabledToolCount = settings.enabledTools.length;
  const primaryActionLabel =
    settings.transport === "http"
      ? status?.runningHttp
        ? "Stop HTTP Server"
        : "Save & Start HTTP Server"
      : "Save MCP Settings";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[760px] max-h-[88vh] overflow-y-auto rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-base font-normal text-[hsl(var(--card-foreground))]">
            MCP Settings
          </DialogTitle>
          <DialogDescription className="text-left text-xs text-muted-foreground">
            Configure the MCP runtime that Infimount exposes locally for external clients.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-5">
          <div className="grid gap-4 rounded-xl border border-border/80 bg-secondary/45 p-4 md:grid-cols-[1.1fr_0.9fr]">
            <div className="space-y-4">
              <div className="rounded-lg border border-border/80 bg-card px-4 py-3 shadow-sm">
                <div>
                  <Label className="text-sm font-medium text-foreground">Transport Settings</Label>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Choose how clients connect. HTTP settings are applied when you start the server.
                  </p>
                </div>
              </div>

              <div className="space-y-2">
                <Label className="text-xs font-normal text-muted-foreground">Transport</Label>
                <Select
                  value={settings.transport}
                  onValueChange={(value) =>
                    setSettings((current) => ({
                      ...current,
                      transport: value as McpSettings["transport"],
                    }))
                  }
                >
                  <SelectTrigger className="border border-border bg-card text-sm text-[hsl(var(--card-foreground))] focus-visible:border-border/80 focus-visible:ring-0 focus-visible:ring-offset-0">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                    <SelectItem value="stdio">stdio</SelectItem>
                    <SelectItem value="http">http</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {settings.transport === "http" ? (
                <div className="space-y-3">
                  <div className="grid gap-4 md:grid-cols-2">
                    <div className="space-y-2">
                      <Label className="text-xs font-normal text-muted-foreground">Bind Address</Label>
                      <Input
                        value={settings.bindAddress}
                        onChange={(event) =>
                          setSettings((current) => ({ ...current, bindAddress: event.target.value }))
                        }
                        className="border border-border bg-card text-sm text-[hsl(var(--card-foreground))] focus-visible:border-border/80 focus-visible:ring-0 focus-visible:ring-offset-0"
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
                          setSettings((current) => ({
                            ...current,
                            port: Number.parseInt(event.target.value || "0", 10),
                          }))
                        }
                        className="border border-border bg-card text-sm text-[hsl(var(--card-foreground))] focus-visible:border-border/80 focus-visible:ring-0 focus-visible:ring-offset-0"
                      />
                    </div>
                  </div>
                  {showNetworkWarning ? (
                    <div className="rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                      This bind address is not loopback. Clients on your LAN may be able to reach this MCP endpoint.
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>

            <div className="space-y-3 rounded-lg border border-border/80 bg-card p-4 shadow-sm">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-sm font-medium text-foreground">Runtime Status</div>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {status?.runningHttp ? "HTTP server is live." : "HTTP server is not running."}
                  </p>
                </div>
                <div
                  className={`rounded-full px-2.5 py-1 text-[10px] font-medium uppercase tracking-[0.14em] ${
                    status?.runningHttp
                      ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                      : "bg-muted text-muted-foreground"
                  }`}
                >
                  {status?.runningHttp ? "Running" : "Stopped"}
                </div>
              </div>

              <div className="rounded-lg border border-border/80 bg-background p-3">
                <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
                  Endpoint
                </div>
                <div className="mt-2 break-all font-mono text-xs text-foreground">{endpointDisplay}</div>
              </div>

              {settings.transport === "http" ? (
                <Button
                  type="button"
                  className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
                  onClick={handleHttpToggle}
                  disabled={isBusy}
                >
                  {status?.runningHttp ? <Square className="mr-2 h-4 w-4" /> : <Play className="mr-2 h-4 w-4" />}
                  {isTogglingHttp ? "Working..." : primaryActionLabel}
                </Button>
              ) : (
                <Button
                  type="button"
                  className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
                  onClick={handleSave}
                  disabled={isBusy}
                >
                  {isSaving ? "Saving..." : primaryActionLabel}
                </Button>
              )}

              {requiresHttpRestart ? (
                <div className="rounded-lg border border-amber-300/80 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                  MCP settings changed. Restart the HTTP server (Stop then Start) to apply these changes.
                </div>
              ) : null}
            </div>
          </div>

          <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <div className="text-sm font-medium text-foreground">Exposed MCP Functions</div>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  Enable only the MCP tools you want to expose. Disabled tools are hidden and cannot be called.
                </p>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="h-8 border-border/80"
                  onClick={() => setSettings((current) => ({
                    ...current,
                    enabledTools: tools.map((tool) => tool.name),
                  }))}
                  disabled={isBusy || tools.length === 0}
                >
                  Enable all
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="h-8 border-border/80"
                  onClick={() => setSettings((current) => ({ ...current, enabledTools: [] }))}
                  disabled={isBusy || tools.length === 0}
                >
                  Disable all
                </Button>
              </div>
            </div>

            <div className="text-xs text-muted-foreground">
              {enabledToolCount} of {tools.length} functions enabled
            </div>

            <div className="max-h-60 space-y-2 overflow-y-auto rounded-lg border border-border/80 bg-background p-2">
              {tools.map((tool) => {
                const checked = settings.enabledTools.includes(tool.name);
                return (
                  <div
                    key={tool.name}
                    className="flex items-start justify-between gap-4 rounded-md border border-transparent px-2 py-2 hover:bg-secondary/50"
                  >
                    <div className="space-y-1">
                      <div className="font-mono text-xs text-foreground">{tool.name}</div>
                      <p className="text-xs text-muted-foreground">{tool.description}</p>
                    </div>
                    <Switch
                      checked={checked}
                      disabled={isBusy}
                      onCheckedChange={(value) =>
                        setSettings((current) => ({
                          ...current,
                          enabledTools: value
                            ? [...current.enabledTools, tool.name].sort()
                            : current.enabledTools.filter((name) => name !== tool.name),
                        }))
                      }
                    />
                  </div>
                );
              })}
              {tools.length === 0 ? (
                <div className="px-2 py-3 text-xs text-muted-foreground">
                  Tool metadata is unavailable.
                </div>
              ) : null}
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <SnippetCard
              title="Stdio Snippet"
              value={snippets?.stdio ?? ""}
              onCopy={() => handleCopy(snippets?.stdio ?? "")}
            />
            <SnippetCard
              title="HTTP Snippet"
              value={snippets?.http ?? ""}
              onCopy={() => handleCopy(snippets?.http ?? "")}
            />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function isLoopbackBindAddress(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return (
    normalized === "127.0.0.1"
    || normalized === "localhost"
    || normalized === "::1"
    || normalized === "[::1]"
  );
}

function sameToolSet(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  const aSorted = [...a].sort();
  const bSorted = [...b].sort();
  return aSorted.every((name, index) => name === bSorted[index]);
}

function SnippetCard({
  title,
  value,
  onCopy,
}: {
  title: string;
  value: string;
  onCopy: () => void;
}) {
  return (
    <div className="space-y-2 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-medium text-foreground">{title}</div>
        <Button
          type="button"
          variant="outline"
          className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
          onClick={onCopy}
          disabled={!value}
        >
          <Copy className="mr-2 h-4 w-4" />
          Copy
        </Button>
      </div>
      <Textarea
        value={value}
        readOnly
        rows={12}
        className="min-h-[220px] border border-border/80 bg-background font-mono text-xs leading-6 text-[hsl(var(--card-foreground))] focus-visible:border-border/80 focus-visible:ring-0 focus-visible:ring-offset-0"
      />
    </div>
  );
}
