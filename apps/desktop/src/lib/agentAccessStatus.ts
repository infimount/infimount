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

  const hasExposedWorkspace = workspaces.some((workspace) => {
    const storage = storages.find((candidate) => candidate.id === workspace.storageId);
    return Boolean(storage?.enabled && storage.mcpExposed);
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

    if (!hasExposedWorkspace) {
      return {
        state: "needs_attention",
        label: "Needs attention",
        detail: "stdio Agent Access is enabled, but no workspace storage is exposed.",
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

  if (!hasExposedWorkspace) {
    return {
      state: "needs_attention",
      label: "Needs attention",
      detail: "No workspace storage is exposed for HTTP Agent Access.",
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
