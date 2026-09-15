import type { McpRuntimeStatus, StorageConfig } from "@/types/storage";
import type { WorkspaceRecord } from "@/lib/api";

export type AgentAccessState =
  | "not_configured"
  | "disabled"
  | "ready_stdio"
  | "http_stopped"
  | "http_running"
  | "needs_attention";

export interface AgentAccessSummary {
  state: AgentAccessState;
  label: string;
  detail: string;
  tone: "neutral" | "success" | "warning";
}

export function isWorkspaceAgentAccessPrepared(
  workspace: WorkspaceRecord,
  storage: StorageConfig | null | undefined,
): boolean {
  if (!storage?.enabled || !storage.mcpExposed || !workspace.policyRuleId) return false;
  return storage.mcpPolicy.rules.some(
    (rule) =>
      rule.id === workspace.policyRuleId &&
      rule.source.kind === "workspace" &&
      rule.source.workspace_id === workspace.id,
  );
}

export function summarizeAgentAccess(
  status: McpRuntimeStatus | null,
  workspaces: WorkspaceRecord[],
  storages: StorageConfig[],
): AgentAccessSummary {
  if (workspaces.length === 0) {
    return {
      state: "not_configured",
      label: "Not configured",
      detail: "Create a workspace to connect an agent.",
      tone: "neutral",
    };
  }

  if (!status) {
    return {
      state: "needs_attention",
      label: "Needs attention",
      detail: "Infimount could not verify Agent Access state.",
      tone: "warning",
    };
  }

  const hasPreparedWorkspace = workspaces.some((workspace) => {
    const storage = storages.find((candidate) => candidate.id === workspace.storageId);
    return isWorkspaceAgentAccessPrepared(workspace, storage);
  });

  if (status.settings.transport === "stdio") {
    if (!status.settings.enabled) {
      return {
        state: "disabled",
        label: "Disabled",
        detail: "General stdio Agent Access is disabled.",
        tone: "neutral",
      };
    }

    if (!hasPreparedWorkspace) {
      return {
        state: "needs_attention",
        label: "Needs attention",
        detail: "stdio Agent Access is enabled, but no workspace has a valid managed access rule.",
        tone: "warning",
      };
    }

    return {
      state: "ready_stdio",
      label: "Ready · stdio on demand",
      detail: "Your MCP client launches Infimount when it connects.",
      tone: "success",
    };
  }

  if (!hasPreparedWorkspace) {
    return {
      state: "needs_attention",
      label: "Needs attention",
      detail: "No workspace has a valid managed access rule for HTTP Agent Access.",
      tone: "warning",
    };
  }

  if (status.runningHttp) {
    return {
      state: "http_running",
      label: "HTTP running",
      detail: status.endpointDisplay,
      tone: "success",
    };
  }

  return {
    state: "http_stopped",
    label: "HTTP stopped",
    detail: "The workspace boundary is ready. Start the HTTP server when you want clients to connect.",
    tone: "warning",
  };
}
