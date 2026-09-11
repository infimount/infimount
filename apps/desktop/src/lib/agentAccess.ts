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

export const WORKSPACE_WRITE_TOOLS = ["mkdir", "write_file", "copy_path"] as const;

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
  // previously drafted HTTP bind. Existing active MCP setups keep their chosen
  // transport and tools; we only add the minimum tools this workspace needs.
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

  return {
    enabled: true,
    transport: status.settings.transport,
    bindAddress: status.settings.bindAddress,
    port: status.settings.port,
    enabledTools: uniqueSorted([...status.settings.enabledTools, ...required]),
    authTokenMutation: { action: "keep" },
  };
}

export async function prepareWorkspaceAgentAccess(
  workspaceId: string,
  accessProfile: string,
): Promise<WorkspaceAgentAccessResult> {
  // Configure the safe runtime/tool surface first. If the later storage exposure
  // check refuses broad/manual policy, no new storage has become visible to MCP.
  const status = await getMcpStatus();
  await updateMcpSettings(workspaceAgentSettings(status, accessProfile));

  try {
    return await invoke<WorkspaceAgentAccessResult>("prepare_workspace_agent_access", {
      workspaceId,
    });
  } catch (error) {
    const code =
      typeof error === "object" && error !== null && "code" in error
        ? String((error as { code: unknown }).code)
        : "UNKNOWN";
    if (code === "ERR_CONFIRMATION_REQUIRED") {
      throw new Error(
        "This storage already has broader MCP grants. Review Advanced MCP settings before exposing it to an agent.",
      );
    }
    if (code === "ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED") {
      throw new Error("The workspace storage identity changed. Recreate the workspace before connecting an agent.");
    }
    if (code === "ERR_WORKSPACE_POLICY_MANAGED") {
      throw new Error("The workspace policy no longer matches this workspace. Recreate or repair the workspace first.");
    }
    throw new Error("Agent access could not be prepared. Review the workspace and retry.");
  }
}
