import { useState, type Dispatch, type SetStateAction } from "react";
import { Copy, ShieldCheck, TestTube2, AlertTriangle } from "lucide-react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type {
  McpClientSnippets,
  McpConfirmationRules,
  McpStoragePolicy,
  McpToolCategory,
  McpToolDefinition,
  McpToolRisk,
  StorageConfig,
  ActiveMcpSession,
  McpSettings,
} from "@/types/storage";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

const SAFE_READ_ONLY_TOOLS = [
  "list_dir",
  "stat_path",
  "read_file",
  "search_paths",
  "list_versions",
  "read_file_version",
];

function needsConfirmation(tool: McpToolDefinition): boolean {
  return (
    tool.category === "write" ||
    tool.category === "destructive" ||
    tool.category === "external_link" ||
    tool.category === "session"
  );
}

function categoryLabel(category: McpToolCategory): string {
  switch (category) {
    case "read": return "Read";
    case "write": return "Write";
    case "destructive": return "Destructive";
    case "external_link": return "External Link";
    case "session": return "Session";
  }
}

function riskStyle(risk: McpToolRisk): string {
  switch (risk) {
    case "low": return "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
    case "medium": return "bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "high": return "bg-red-500/10 text-red-700 dark:text-red-300";
  }
}

function riskLabel(risk: McpToolRisk): string {
  switch (risk) {
    case "low": return "Low risk";
    case "medium": return "Medium risk";
    case "high": return "High risk";
  }
}

const CATEGORY_ORDER: McpToolCategory[] = ["read", "write", "destructive", "external_link", "session"];

interface McpAccessPreset {
  id: string;
  title: string;
  description: string;
  recommendedFor: string;
  enabledTools: string[];
  accessMode: McpStoragePolicy["default_access"];
  confirmationRules: McpConfirmationRules;
}

const DEFAULT_CONFIRMATION_RULES: McpConfirmationRules = {
  require_for_write: true,
  require_for_overwrite: true,
  require_for_delete: true,
  require_for_version_delete: true,
  require_for_presign: true,
  require_for_cross_storage_copy: true,
};

const MCP_ACCESS_PRESETS: McpAccessPreset[] = [
  {
    id: "research-read-only",
    title: "Read-only research",
    description: "Agents can browse, inspect, and search exposed storage without mutations.",
    recommendedFor: "Research, summarization, and read-only assistants.",
    enabledTools: SAFE_READ_ONLY_TOOLS,
    accessMode: "read_only",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
  {
    id: "workspace-agent",
    title: "Workspace Agent",
    description: "Enable non-destructive write tools while preserving each workspace grant.",
    recommendedFor: "Coding and data-analysis agents working inside explicit workspace roots.",
    enabledTools: [...SAFE_READ_ONLY_TOOLS, "mkdir", "write_file", "copy_path"],
    accessMode: "read_write",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
  {
    id: "manual-approval",
    title: "Manual Approval",
    description: "Keep all read tools enabled and require explicit confirmation for every write, delete, and link operation.",
    recommendedFor: "Controlled environments where every agent mutation should be reviewed.",
    enabledTools: [...SAFE_READ_ONLY_TOOLS, "write_file", "mkdir", "delete_path", "copy_path", "move_path", "generate_download_link", "delete_version", "session_create", "session_end"],
    accessMode: "read_write",
    confirmationRules: {
      require_for_write: true,
      require_for_overwrite: true,
      require_for_delete: true,
      require_for_version_delete: true,
      require_for_presign: true,
      require_for_cross_storage_copy: true,
    },
  },
  {
    id: "locked-down",
    title: "Lock down MCP",
    description: "Disable all tools and set exposed storage policies to no access. Existing path rules are preserved but overridden by default none.",
    recommendedFor: "Pausing MCP exposure before changing clients, networks, or policies.",
    enabledTools: [],
    accessMode: "none",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
];

function filterAvailableTools(toolNames: string[], tools: McpToolDefinition[]): string[] {
  const available = new Set(tools.map((tool) => tool.name));
  return toolNames.filter((name) => available.has(name));
}

function formatStorageCount(count: number): string {
  return `${count} ${count === 1 ? "storage" : "storages"}`;
}

function describeStorageAccess(
  storage: StorageConfig,
  policy: McpStoragePolicy,
): { label: string; className: string } {
  if (policy.default_access === "none") {
    return {
      label: "no access",
      className: "bg-muted text-muted-foreground",
    };
  }
  if (storage.readOnly || policy.default_access === "read_only") {
    return {
      label: "read-only",
      className: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
    };
  }
  return {
    label: "read/write",
    className: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  };
}

function describeStorageConfirmations(policy: McpStoragePolicy): string {
  const rules = policy.confirmation_rules;
  const enabled = [
    rules.require_for_write || rules.require_for_overwrite,
    rules.require_for_delete || rules.require_for_version_delete,
    rules.require_for_presign,
    rules.require_for_cross_storage_copy,
  ].filter(Boolean).length;

  if (enabled === 0) return "no approvals";
  return `${enabled} approval ${enabled === 1 ? "rule" : "rules"}`;
}

function SummaryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background px-3 py-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-right text-foreground">{value}</span>
    </div>
  );
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
        className={`min-h-[220px] border border-border/80 bg-background font-mono text-xs leading-6 text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
      />
    </div>
  );
}

interface McpToolSectionProps {
  tools: McpToolDefinition[];
  settings: McpSettings;
  onSettingsChange: Dispatch<SetStateAction<McpSettings>>;
  isBusy: boolean;
  snippets: McpClientSnippets | null;
  onCopy: (text: string) => void;
  exposedStorages: StorageConfig[];
  policyDrafts: Record<string, McpStoragePolicy>;
  onApplyPreset: (preset: McpAccessPreset) => void;
  applyingPresetId: string | null;
  connectAssessment: { label: string; description: string; className: string };
  accessCounts: { readWrite: number; readOnly: number; noAccess: number };
  readAccessSummary: string;
  writeAccessSummary: string;
  destructiveAccessSummary: string;
  presignSummary: string;
  confirmationSummary: string;
  showNetworkWarning: boolean;
  activeSessions: ActiveMcpSession[];
  onTestServer: () => Promise<void>;
}

export function McpToolSection({
  tools,
  settings,
  onSettingsChange,
  isBusy,
  snippets,
  onCopy,
  exposedStorages,
  policyDrafts,
  onApplyPreset,
  applyingPresetId,
  connectAssessment,
  accessCounts,
  readAccessSummary,
  writeAccessSummary,
  destructiveAccessSummary,
  presignSummary,
  confirmationSummary,
  showNetworkWarning,
  activeSessions,
  onTestServer,
}: McpToolSectionProps) {
  const enabledToolCount = settings.enabledTools.length;
  const [pendingRiskyTool, setPendingRiskyTool] = useState<McpToolDefinition | null>(null);
  const [pendingPreset, setPendingPreset] = useState<McpAccessPreset | null>(null);
  const [showAdvancedTools, setShowAdvancedTools] = useState(false);

  const safeTools = tools.filter((t) => t.defaultEnabled);
  const advancedTools = tools.filter((t) => !t.defaultEnabled);

  const groupedTools = CATEGORY_ORDER
    .map((cat) => ({
      category: cat,
      label: categoryLabel(cat),
      tools: advancedTools.filter((t) => t.category === cat),
    }))
    .filter((g) => g.tools.length > 0);

  const handleToolToggle = (tool: McpToolDefinition, checked: boolean) => {
    if (checked && needsConfirmation(tool)) {
      setPendingRiskyTool(tool);
      return;
    }

    applyToolToggle(tool, checked);
  };

  const applyToolToggle = (tool: McpToolDefinition, checked: boolean) => {
    onSettingsChange((current) => ({
      ...current,
      enabledTools: checked
        ? [...current.enabledTools, tool.name].sort()
        : current.enabledTools.filter((name) => name !== tool.name),
    }));
    setPendingRiskyTool(null);
  };

  const handleApplySafeReadOnly = () => {
    onSettingsChange((current) => ({
      ...current,
      enabledTools: filterAvailableTools(SAFE_READ_ONLY_TOOLS, tools),
    }));
  };

  return (
    <>
      <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="text-sm font-medium text-foreground">Exposed MCP Functions</div>
            <p className="mt-1 text-[11px] text-muted-foreground">
              Enable only the MCP tools you want to expose. Disabled tools are hidden and
              cannot be called.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 border-border/80"
              onClick={handleApplySafeReadOnly}
              disabled={isBusy || tools.length === 0}
            >
              Apply safe read-only
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 border-border/80"
              aria-expanded={showAdvancedTools}
              onClick={() => setShowAdvancedTools((current) => !current)}
              disabled={isBusy || groupedTools.length === 0}
            >
              Configure advanced tools
            </Button>
          </div>
        </div>

        <div className="text-xs text-muted-foreground">
          {enabledToolCount} of {tools.length} functions enabled
        </div>

        <div className="max-h-60 space-y-1 overflow-y-auto rounded-lg border border-border/80 bg-background p-2">
          <div className="px-2 py-1.5">
            <div className="text-[11px] font-medium text-muted-foreground uppercase tracking-wide">
              Safe read-only tools
            </div>
          </div>
          {safeTools.map((tool) => {
            const checked = settings.enabledTools.includes(tool.name);
            return (
              <div
                key={tool.name}
                className="flex items-start justify-between gap-4 rounded-md border border-transparent px-2 py-2 hover:bg-secondary/50"
              >
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs text-foreground">{tool.name}</span>
                    <span className={`rounded-full px-1.5 py-0.5 text-[10px] ${riskStyle(tool.risk)}`}>
                      {riskLabel(tool.risk)}
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground">{tool.description}</p>
                </div>
                <Switch
                  aria-label={`Enable ${tool.name}`}
                  checked={checked}
                  disabled={isBusy}
                  onCheckedChange={(value) => handleToolToggle(tool, value)}
                />
              </div>
            );
          })}

          {showAdvancedTools && groupedTools.length > 0 && (
            <>
              <div className="mt-3 border-t border-border/60 pt-2">
                <div className="px-2 py-1.5 flex items-center gap-2">
                  <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
                  <span className="text-[11px] font-medium text-muted-foreground uppercase tracking-wide">
                    Advanced tools (disabled by default)
                  </span>
                </div>
              </div>
              {groupedTools.map((group) => (
                <div key={group.category}>
                  <div className="px-2 py-1">
                    <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
                      {group.label}
                    </span>
                  </div>
                  {group.tools.map((tool) => {
                    const checked = settings.enabledTools.includes(tool.name);
                    return (
                      <div
                        key={tool.name}
                        className="flex items-start justify-between gap-4 rounded-md border border-transparent px-2 py-2 hover:bg-secondary/50"
                      >
                        <div className="space-y-1">
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-xs text-foreground">{tool.name}</span>
                            <span className={`rounded-full px-1.5 py-0.5 text-[10px] ${riskStyle(tool.risk)}`}>
                              {riskLabel(tool.risk)}
                            </span>
                          </div>
                          <p className="text-xs text-muted-foreground">{tool.description}</p>
                        </div>
                        <Switch
                          aria-label={`Enable ${tool.name}`}
                          checked={checked}
                          disabled={isBusy}
                          onCheckedChange={(value) => handleToolToggle(tool, value)}
                        />
                      </div>
                    );
                  })}
                </div>
              ))}
            </>
          )}

          {tools.length === 0 ? (
            <div className="px-2 py-3 text-xs text-muted-foreground">
              Tool metadata is unavailable.
            </div>
          ) : null}
        </div>
      </div>

      <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
        <div>
          <div className="text-sm font-medium text-foreground">MCP Access Presets</div>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            Start from a safe operating mode. Presets save tool visibility and exposed storage
            policies, but they do not expose hidden storages.
          </p>
        </div>
        <div className="grid gap-2 md:grid-cols-2">
          {MCP_ACCESS_PRESETS.map((preset) => (
            <button
              key={preset.id}
              type="button"
              className="rounded-lg border border-border/80 bg-background px-3 py-3 text-left transition-colors hover:bg-secondary/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => {
                const enablesRiskyTools = preset.enabledTools.some((name) => {
                  const tool = tools.find((candidate) => candidate.name === name);
                  return tool ? needsConfirmation(tool) : false;
                });
                if (enablesRiskyTools) {
                  setPendingPreset(preset);
                } else {
                  void onApplyPreset(preset);
                }
              }}
              disabled={isBusy}
            >
              <span className="flex items-start justify-between gap-3">
                <span>
                  <span className="block text-sm font-medium text-foreground">
                    {preset.title}
                  </span>
                  <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                    {preset.description}
                  </span>
                  <span className="mt-1 block text-[11px] leading-5 text-muted-foreground">
                    Best for: {preset.recommendedFor}
                  </span>
                </span>
                <span className="rounded-full border border-border bg-card px-2 py-0.5 text-[10px] text-muted-foreground">
                  {preset.accessMode === "read_write"
                    ? "read/write"
                    : preset.accessMode === "read_only"
                      ? "read-only"
                      : "no access"}
                </span>
              </span>
              <span className="mt-2 block text-[11px] text-muted-foreground">
                {applyingPresetId === preset.id
                  ? "Applying..."
                  : `${filterAvailableTools(preset.enabledTools, tools).length} tools, ${formatStorageCount(exposedStorages.length)}`}
              </span>
            </button>
          ))}
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <SnippetCard
          title="Stdio Snippet"
          value={snippets?.stdio ?? ""}
          onCopy={() => onCopy(snippets?.stdio ?? "")}
        />
        <SnippetCard
          title="HTTP Snippet"
          value={snippets?.http ?? ""}
          onCopy={() => onCopy(snippets?.http ?? "")}
        />
      </div>

      <div className="grid gap-4 md:grid-cols-[0.9fr_1.1fr]">
        <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
          <div className="flex items-start gap-3">
            <ShieldCheck className="mt-0.5 h-4 w-4 text-primary" />
            <div>
              <div className="text-sm font-medium text-foreground">
                What the agent can access
              </div>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                This summary reflects enabled storages and currently enabled MCP functions.
              </p>
            </div>
          </div>
          <div className="rounded-lg border border-border/70 bg-background px-3 py-2 text-xs">
            <div className="flex items-center justify-between gap-3">
              <span className="text-muted-foreground">Safe to connect?</span>
              <span className={`text-right font-medium ${connectAssessment.className}`}>
                {connectAssessment.label}
              </span>
            </div>
            <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
              {connectAssessment.description}
            </p>
          </div>
          <div className="space-y-2 text-xs">
            <SummaryRow label="Exposed storages" value={`${exposedStorages.length}`} />
            <SummaryRow label="Enabled functions" value={`${enabledToolCount}`} />
            <SummaryRow label="Active sessions" value={`${activeSessions.length}`} />
            <SummaryRow
              label="Access levels"
              value={`${accessCounts.readWrite} write / ${accessCounts.readOnly} read-only / ${accessCounts.noAccess} no access`}
            />
            <SummaryRow label="Read access" value={readAccessSummary} />
            <SummaryRow label="Write access" value={writeAccessSummary} />
            <SummaryRow label="Destructive access" value={destructiveAccessSummary} />
            <SummaryRow label="Presigned links" value={presignSummary} />
            <SummaryRow label="Risk confirmations" value={confirmationSummary} />
            {showNetworkWarning ? (
              <SummaryRow label="Network exposure" value="Non-loopback bind needs review" />
            ) : null}
          </div>
        </div>

        <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="text-sm font-medium text-foreground">MCP Setup Wizard</div>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                Confirm runtime status, enabled functions, and exposed storage count before
                connecting an agent.
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              className="border-border/80"
              onClick={() => void onTestServer()}
              disabled={isBusy}
            >
              <TestTube2 className="mr-2 h-4 w-4" />
              Test
            </Button>
          </div>
          <div className="rounded-lg border border-border/80 bg-background p-3">
            <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
              Exposed Storages
            </div>
            <div className="mt-2 flex flex-wrap gap-1.5">
              {exposedStorages.length > 0 ? (
                exposedStorages.map((storage) => {
                  const policy = policyDrafts[storage.id] ?? storage.mcpPolicy;
                  const access = describeStorageAccess(storage, policy);
                  const confirmations = describeStorageConfirmations(policy);
                  return (
                    <span
                      key={storage.id}
                      className="inline-flex flex-wrap items-center gap-1.5 rounded-lg border border-border bg-secondary/50 px-2 py-1 text-xs text-foreground"
                    >
                      <span className="font-medium">{storage.name}</span>
                      <span className={`rounded-full px-1.5 py-0.5 text-[10px] ${access.className}`}>
                        {access.label}
                      </span>
                      <span className="rounded-full bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {confirmations}
                      </span>
                    </span>
                  );
                })
              ) : (
                <span className="text-xs text-muted-foreground">
                  No storage is currently exposed to MCP.
                </span>
              )}
            </div>
          </div>
        </div>
      </div>

      <AlertDialog
        open={pendingPreset !== null}
        onOpenChange={(open) => {
          if (!open) setPendingPreset(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Apply {pendingPreset?.title}?</AlertDialogTitle>
            <AlertDialogDescription>
              This preset enables advanced tools that can modify data, create external links, or
              change scoped sessions. Existing path grants remain authoritative and confirmation
              rules stay enabled.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (pendingPreset) void onApplyPreset(pendingPreset);
                setPendingPreset(null);
              }}
            >
              Apply preset
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={pendingRiskyTool !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRiskyTool(null);
        }}
      >
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>Enable {pendingRiskyTool?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              This is a{" "}
              <span className="font-medium">
                {pendingRiskyTool?.category === "destructive"
                  ? "destructive"
                  : `${pendingRiskyTool?.risk ?? "unknown"}-risk ${pendingRiskyTool?.category?.replace("_", "-") ?? "advanced"}`}
              </span>{" "}
              tool. Enabling it may allow agents to modify or delete data, create external links,
              or change scoped session state. Make sure your storage policies and confirmation
              rules match your safety requirements.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={() => {
                if (pendingRiskyTool) {
                  applyToolToggle(pendingRiskyTool, true);
                }
              }}
            >
              Enable
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}