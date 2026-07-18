import type { Dispatch, SetStateAction } from "react";
import { Copy, ShieldCheck, TestTube2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type {
  McpClientSnippets,
  McpConfirmationRules,
  McpStoragePolicy,
  McpToolDefinition,
  StorageConfig,
  ActiveMcpSession,
  McpSettings,
} from "@/types/storage";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

const READ_ONLY_TOOLS = [
  "list_dir",
  "stat_path",
  "read_file",
  "search_paths",
  "list_storages",
  "validate_storage",
  "list_versions",
  "read_file_version",
];

const WORKSPACE_TOOLS = [
  ...READ_ONLY_TOOLS,
  "mkdir",
  "write_file",
  "copy_path",
  "move_path",
];

const FULL_MANUAL_TOOLS = [
  ...WORKSPACE_TOOLS,
  "delete_path",
  "generate_download_link",
  "delete_version",
  "session_create",
  "session_end",
];

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
    enabledTools: READ_ONLY_TOOLS,
    accessMode: "read_only",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
  {
    id: "workspace-agent",
    title: "Workspace agent",
    description: "Agents can create, write, copy, and move files while risky work still requires approval.",
    recommendedFor: "Coding agents working inside an allowed project or workspace root.",
    enabledTools: WORKSPACE_TOOLS,
    accessMode: "read_write",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
  {
    id: "manual-approval",
    title: "Manual approval mode",
    description: "Broad file tools are available, but writes, deletes, links, and cross-storage work wait for approval.",
    recommendedFor: "Data cleanup, backups, and operator workflows with a human in the loop.",
    enabledTools: FULL_MANUAL_TOOLS,
    accessMode: "read_write",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
  {
    id: "locked-down",
    title: "Lock down MCP",
    description: "Disable all tools and set exposed storage policies to no access.",
    recommendedFor: "Pausing MCP exposure before changing clients, networks, or policies.",
    enabledTools: [],
    accessMode: "none",
    confirmationRules: DEFAULT_CONFIRMATION_RULES,
  },
];

function filterAvailableTools(toolNames: string[], tools: McpToolDefinition[]): string[] {
  const available = new Set(tools.map((tool) => tool.name));
  return toolNames.filter((name) => available.has(name)).sort();
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
              onClick={() =>
                onSettingsChange((current) => ({
                  ...current,
                  enabledTools: tools.map((tool) => tool.name),
                }))
              }
              disabled={isBusy || tools.length === 0}
            >
              Enable all
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 border-border/80"
              onClick={() =>
                onSettingsChange((current) => ({ ...current, enabledTools: [] }))
              }
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
                    onSettingsChange((current) => ({
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
              onClick={() => void onApplyPreset(preset)}
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
    </>
  );
}
