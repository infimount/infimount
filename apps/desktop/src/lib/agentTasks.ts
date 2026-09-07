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

export interface AgentTaskSummary {
  taskId: string;
  title: string;
  createdAt: string;
  workspaceId: string;
  taskRoot: string;
  preparedFiles: number;
  preparedBytes: number;
}

export interface AgentTaskOutputEntry {
  taskPath: string;
  byteSize: number;
  sha256: string;
  modifiedAt: string | null;
}

export interface AgentTaskOutputList {
  task: AgentTaskSummary;
  outputs: AgentTaskOutputEntry[];
}

export interface ReviewedAgentTaskOutput {
  taskPath: string;
  sha256: string;
}

export interface PublishAgentTaskRequest {
  workspaceId: string;
  taskId: string;
  outputs: ReviewedAgentTaskOutput[];
  destinationStorageId: string;
  destinationDir: string;
}

export type PublishAgentTaskStatus = "published" | "conflict" | "stale" | "failed";

export interface PublishAgentTaskItemResult {
  taskPath: string;
  destinationPath: string;
  sha256: string;
  byteSize: number;
  status: PublishAgentTaskStatus;
  errorCode: string | null;
}

export interface PublishAgentTaskResult {
  taskId: string;
  published: number;
  conflicts: number;
  stale: number;
  failed: number;
  receiptWritten: boolean;
  results: PublishAgentTaskItemResult[];
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

async function invokeTask<T>(command: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    const safe = sanitizeTaskError(error);
    throw new TauriApiError(safe.message, safe.code);
  }
}

export function preflightAgentTask(
  request: AgentTaskPreparationRequest,
): Promise<AgentTaskPreflightOutput> {
  return invokeTask<AgentTaskPreflightOutput>("preflight_agent_task", { request });
}

export function prepareAgentTask(
  request: AgentTaskPreparationRequest,
): Promise<PrepareAgentTaskOutput> {
  return invokeTask<PrepareAgentTaskOutput>("prepare_agent_task", { request });
}

export function listAgentTasks(workspaceId: string): Promise<AgentTaskSummary[]> {
  return invokeTask<AgentTaskSummary[]>("list_agent_tasks", { workspaceId });
}

export function listAgentTaskOutputs(
  workspaceId: string,
  taskId: string,
): Promise<AgentTaskOutputList> {
  return invokeTask<AgentTaskOutputList>("list_agent_task_outputs", { workspaceId, taskId });
}

export function publishAgentTaskOutputs(
  request: PublishAgentTaskRequest,
): Promise<PublishAgentTaskResult> {
  return invokeTask<PublishAgentTaskResult>("publish_agent_task_outputs", { request });
}

export function normalizeRequestedOutputs(value: string): string[] {
  const seen = new Set<string>();
  const outputs: string[] = [];
  for (const raw of value.split(/\r?\n|,/)) {
    const trimmed = raw.trim().replace(/^\/+/, "");
    if (!trimmed) continue;
    const path = trimmed.startsWith("outputs/") ? trimmed : `outputs/${trimmed}`;
    const key = path.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    outputs.push(path);
  }
  return outputs;
}

export function formatTaskBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}
