import { invoke } from "@tauri-apps/api/core";

import { getMcpStatus, updateMcpSettings } from "@/lib/api";
import type { McpRuntimeStatus, McpSettingsUpdate } from "@/types/storage";

export const WORKSPACE_READ_TOOLS = [
  "list_dir",
  "stat_path",
  "read_file",
  "search_paths",
  "list_versions",
  "read_file_version",
] as const;

export const WORKSPACE_WRITE_TOOLS = ["mkdir", "write_file"] as const;

export interface WorkspaceAgentAccessResult {
  workspaceId: string;
  storageId: string;
  accessProfile: string;
  mcpExposed: boolean;
  changed: boolean;
}

function uniqueSorted(values: string[]): string[] {
  return Array.from(new Set(values)).sort((left, right) => left.localeCompare(right));
}

export function workspaceAgentSettings(
  status: McpRuntimeStatus,
  accessProfile: string,
): McpSettingsUpdate {
  const required = [
    ...WORKSPACE_READ_TOOLS,
    ...(accessProfile === "read_write" ? WORKSPACE_WRITE_TOOLS : []),
  ];

  // First-time setup intentionally chooses local stdio rather than activating a
  // previously drafted HTTP bind.
  if (!status.settings.enabled) {
    return {
      enabled: true,
      transport: "stdio",
      bindAddress: "127.0.0.1",
      port: status.settings.port,
      enabledTools: uniqueSorted(required),
      authTokenMutation: { action: "keep" },
    };
  }

  // Exposing a new workspace while the active server has additional global
  // tools would silently give that workspace more capability than this guided
  // step presents. Do not remove an advanced user's tools behind their back;
  // fail closed and send that configuration to the Advanced MCP surface.
  const requiredSet = new Set<string>(required);
  const additionalTools = status.settings.enabledTools.filter((tool) => !requiredSet.has(tool));
  if (additionalTools.length > 0) {
    throw new Error(
      "MCP already has additional tools enabled. Review Advanced MCP settings before exposing this workspace.",
    );
  }

  return {
    enabled: true,
    transport: status.settings.transport,
    bindAddress: status.settings.bindAddress,
    port: status.settings.port,
    enabledTools: uniqueSorted([...status.settings.enabledTools, ...required]),
    authTokenMutation: { action: "keep" },
  };
}

function agentAccessError(error: unknown): Error {
  const code =
    typeof error === "object" && error !== null && "code" in error
      ? String((error as { code: unknown }).code)
      : "UNKNOWN";
  if (code === "ERR_CONFIRMATION_REQUIRED") {
    return new Error(
      "This storage already has broader MCP grants. Review Advanced MCP settings before exposing it to an agent.",
    );
  }
  if (code === "ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED") {
    return new Error(
      "The workspace storage identity changed. Recreate the workspace before connecting an agent.",
    );
  }
  if (code === "ERR_WORKSPACE_POLICY_MANAGED") {
    return new Error(
      "The workspace policy no longer matches this workspace. Recreate or repair the workspace first.",
    );
  }
  if (error instanceof Error) return error;
  return new Error("Agent access could not be prepared. Review the workspace and retry.");
}

export async function prepareWorkspaceAgentAccess(
  workspaceId: string,
  _requestedAccessProfile?: string,
): Promise<WorkspaceAgentAccessResult> {
  try {
    // Preflight is read-only. It rejects stale namespace bindings and broad/manual
    // grants before onboarding changes the MCP runtime at all. The validated
    // backend record is also the source of truth for the required tool profile;
    // callers cannot broaden tools by supplying a stale or incorrect profile.
    const preflight = await invoke<WorkspaceAgentAccessResult>("check_workspace_agent_access", {
      workspaceId,
    });

    const status = await getMcpStatus();
    await updateMcpSettings(workspaceAgentSettings(status, preflight.accessProfile));

    // The commit path repeats every preflight check under the configuration lock
    // before exposing the backing storage, so policy drift cannot race the UI.
    return await invoke<WorkspaceAgentAccessResult>("prepare_workspace_agent_access", {
      workspaceId,
    });
  } catch (error) {
    throw agentAccessError(error);
  }
}
