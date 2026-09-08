import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { TauriApiError } from "@/lib/api";

export interface AgentTaskPreparationRequest {
  sourceStorageId: string;
  sourcePaths: string[];
  workspaceId: string;
  title: string;
  objective: string;
  requestedOutputs: string[];
}

export interface AgentTaskCodexHandoffRequest {
  workspaceId: string;
  taskId: string;
}

export interface AgentTaskPreflightOutput {
  workspaceId: string;
  workspaceName: string;
  selectedItems: number;
  fileCount: number;
  directoryCount: number;
  totalBytes: number;
  sourceMcpExposed: boolean;
  workspaceMcpExposed: boolean;
  warnings: string[];
}

export interface PrepareAgentTaskOutput {
  taskId: string;
  taskRoot: string;
  workspaceId: string;
  workspaceName: string;
  preparedFiles: number;
  totalBytes: number;
  sourceMcpExposed: boolean;
  workspaceMcpExposed: boolean;
}

export interface AgentTaskCodexHandoffOutput {
  client: "codex";
  workspaceId: string;
  workspaceName: string;
  taskId: string;
  taskRoot: string;
  launched: boolean;
}

function sanitizeTaskError(value: unknown): { code: string; message: string } {
  const record = typeof value === "object" && value !== null ? value as Record<string, unknown> : null;
  const code = record && typeof record.code === "string" ? record.code : "UNKNOWN";
  const raw = record && typeof record.message === "string"
    ? record.message
    : value instanceof Error
      ? value.message
      : "Agent Task request failed.";
  const suspicious = /(https?:\/\/|authorization|bearer|token|secret|password|client[_ ]?secret|access[_ ]?key|[?&][^\s=]+=)/i;
  const message = suspicious.test(raw) ? "Agent Task request failed." : raw.slice(0, 320);
  return { code, message };
}

async function invokeTask<T>(
  command: string,
  request: AgentTaskPreparationRequest | AgentTaskCodexHandoffRequest,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, { request });
  } catch (error) {
    const safe = sanitizeTaskError(error);
    throw new TauriApiError(safe.message, safe.code);
  }
}

export function preflightAgentTask(
  request: AgentTaskPreparationRequest,
): Promise<AgentTaskPreflightOutput> {
  return invokeTask<AgentTaskPreflightOutput>("preflight_agent_task", request);
}

export function prepareAgentTask(
  request: AgentTaskPreparationRequest,
): Promise<PrepareAgentTaskOutput> {
  return invokeTask<PrepareAgentTaskOutput>("prepare_agent_task", request);
}

export function launchAgentTaskInCodex(
  request: AgentTaskCodexHandoffRequest,
): Promise<AgentTaskCodexHandoffOutput> {
  return invokeTask<AgentTaskCodexHandoffOutput>("launch_agent_task_in_codex", request);
}
