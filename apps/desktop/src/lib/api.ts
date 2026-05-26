import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpStoragePolicy,
  McpToolDefinition,
  PendingMcpConfirmation,
  AppSettings,
  McpAuditEvent,
  StorageCapabilities,
  StorageConfig,
  StorageDraft,
  StorageValidationResult,
} from "@/types/storage";

export interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  modified_at: string | null;
}

export interface ApiError {
  code: string;
  message: string;
}

export class TauriApiError extends Error {
  code: string;

  constructor(message: string, code = "UNKNOWN") {
    super(message);
    this.name = "TauriApiError";
    this.code = code;
  }
}

export interface StorageFieldSchema {
  name: string;
  label: string;
  input_type?: string;
  required?: boolean;
  secret?: boolean;
}

export interface StorageKindSchema {
  id: string;
  label: string;
  kind: string;
  fields: StorageFieldSchema[];
}

export type TransferOperation = "copy" | "move";
export type TransferConflictPolicy = "fail" | "overwrite" | "skip" | "rename";
export type TransferPlanAction = "create" | "overwrite" | "skip" | "rename" | "noop" | "conflict";

export interface TransferPlanEntry {
  sourcePath: string;
  destinationPath: string;
  isDir: boolean;
  size: number;
  action: TransferPlanAction;
}

export interface TransferPlanSummary {
  create: number;
  overwrite: number;
  skip: number;
  rename: number;
  noop: number;
  conflict: number;
  totalItems: number;
  totalBytes: number;
}

export interface TransferPlan {
  operation: TransferOperation;
  conflictPolicy: TransferConflictPolicy;
  entries: TransferPlanEntry[];
  summary: TransferPlanSummary;
}

export interface ImportStoragesRequest {
  json: string;
  mode: "merge" | "replace";
  onConflict: "error" | "overwrite" | "rename";
}

export interface ImportStoragesResult {
  imported: number;
}

export interface ExportStoragesResult {
  json: string;
}

export interface FileVersion {
  version: string;
  size_bytes: number | null;
  modified_at: string | null;
  etag: string | null;
}

export interface ListVersionsResult {
  path: string;
  versions: FileVersion[];
  next_cursor: string | null;
}

export interface DeleteVersionResult {
  path: string;
  version: string;
  deleted: boolean;
}

async function handleError(error: unknown): Promise<never> {
  console.error("API Error:", error);
  if (typeof error === "object" && error !== null && "code" in error && "message" in error) {
    const apiErr = error as { code: string; message: string };
    throw new TauriApiError(apiErr.message, apiErr.code);
  }

  const message =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : String(error);
  throw new TauriApiError(message);
}

async function invokeOrThrow<T>(command: string): Promise<T>;
async function invokeOrThrow<T>(command: string, args: Record<string, unknown>): Promise<T>;
async function invokeOrThrow<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return args === undefined ? await tauriInvoke<T>(command) : await tauriInvoke<T>(command, args);
  } catch (error) {
    return handleError(error);
  }
}

export function listEntries(sourceId: string, path: string): Promise<Entry[]> {
  return invokeOrThrow<Entry[]>("list_entries", { sourceId, path });
}

export function listEntriesRecursive(sourceId: string, path = "/"): Promise<Entry[]> {
  return invokeOrThrow<Entry[]>("list_entries_recursive", { sourceId, path });
}

export function statEntry(sourceId: string, path: string): Promise<Entry> {
  return invokeOrThrow<Entry>("stat_entry", { sourceId, path });
}

export async function readFile(sourceId: string, path: string): Promise<Uint8Array> {
  const data = await invokeOrThrow<number[]>("read_file", { sourceId, path });
  return new Uint8Array(data);
}

export function writeFile(
  sourceId: string,
  path: string,
  data: Uint8Array,
  userMetadata?: Record<string, string>,
): Promise<void> {
  const args: { sourceId: string; path: string; data: number[]; userMetadata?: Record<string, string> } = {
    sourceId,
    path,
    data: Array.from(data),
  };
  if (userMetadata && Object.keys(userMetadata).length > 0) {
    args.userMetadata = userMetadata;
  }
  return invokeOrThrow<void>("write_file", args);
}

export function createDirectory(sourceId: string, path: string): Promise<void> {
  return invokeOrThrow<void>("create_directory", { sourceId, path });
}

export function uploadDroppedFiles(
  sourceId: string,
  paths: string[],
  targetDir: string,
): Promise<void> {
  return invokeOrThrow<void>("upload_dropped_files", { sourceId, paths, targetDir });
}

export function deletePath(sourceId: string, path: string): Promise<void> {
  return invokeOrThrow<void>("delete_path", { sourceId, path });
}

export function planTransferEntries(
  fromSourceId: string,
  toSourceId: string,
  paths: string[],
  targetDir: string,
  operation: TransferOperation,
  conflictPolicy: TransferConflictPolicy,
): Promise<TransferPlan> {
  return invokeOrThrow<TransferPlan>("plan_transfer_entries", {
    fromSourceId,
    toSourceId,
    paths,
    targetDir,
    operation,
    conflictPolicy,
  });
}

export function transferEntries(
  fromSourceId: string,
  toSourceId: string,
  paths: string[],
  targetDir: string,
  operation: TransferOperation,
  conflictPolicy: TransferConflictPolicy,
  jobId?: string,
): Promise<void> {
  return invokeOrThrow<void>("transfer_entries", {
    fromSourceId,
    toSourceId,
    paths,
    targetDir,
    operation,
    conflictPolicy,
    ...(jobId ? { jobId } : {}),
  });
}

export function cancelTransferJob(jobId: string): Promise<void> {
  return invokeOrThrow<void>("cancel_transfer_job", { jobId });
}

export function listStorages(): Promise<StorageConfig[]> {
  return invokeOrThrow<StorageConfig[]>("list_storages");
}

export function addStorage(storage: StorageDraft): Promise<StorageConfig> {
  return invokeOrThrow<StorageConfig>("add_storage", { storage });
}

export function updateStorage(storageId: string, storage: StorageDraft): Promise<StorageConfig> {
  return invokeOrThrow<StorageConfig>("update_storage", { storageId, storage });
}

export function removeStorage(storageId: string): Promise<void> {
  return invokeOrThrow<void>("remove_storage", { storageId });
}

export function updateMcpStoragePolicy(
  storageId: string,
  policy: McpStoragePolicy,
): Promise<StorageConfig> {
  return invokeOrThrow<StorageConfig>("update_mcp_storage_policy", { storageId, policy });
}

export function verifyStorage(storage: StorageDraft): Promise<StorageValidationResult> {
  return invokeOrThrow<StorageValidationResult>("verify_storage", { storage });
}

export function importStorageConfig(request: ImportStoragesRequest): Promise<ImportStoragesResult> {
  return invokeOrThrow<ImportStoragesResult>("import_storage_config", {
    request: {
      json: request.json,
      mode: request.mode,
      onConflict: request.onConflict,
    },
  });
}

export function exportStorageConfig(includeSecrets: boolean): Promise<ExportStoragesResult> {
  return invokeOrThrow<ExportStoragesResult>("export_storage_config", { includeSecrets });
}

export function listStorageSchemas(): Promise<StorageKindSchema[]> {
  return invokeOrThrow<StorageKindSchema[]>("list_storage_schemas");
}

export function getStorageCapabilities(storageId: string): Promise<StorageCapabilities> {
  return invokeOrThrow<StorageCapabilities>("get_storage_capabilities", { storageId });
}

export function generateDownloadLink(
  sourceId: string,
  path: string,
  expiresSeconds = 900,
): Promise<string> {
  return invokeOrThrow<string>("generate_download_link", { sourceId, path, expiresSeconds });
}

export function getAppSettings(): Promise<AppSettings> {
  return invokeOrThrow<AppSettings>("get_app_settings");
}

export function completeOnboarding(): Promise<AppSettings> {
  return invokeOrThrow<AppSettings>("complete_onboarding");
}

export function skipOnboarding(): Promise<AppSettings> {
  return invokeOrThrow<AppSettings>("skip_onboarding");
}

export function listMcpAuditEvents(limit = 200): Promise<McpAuditEvent[]> {
  return invokeOrThrow<McpAuditEvent[]>("list_mcp_audit_events", { limit });
}

export function clearMcpAuditEvents(): Promise<void> {
  return invokeOrThrow<void>("clear_mcp_audit_events");
}

export function listPendingMcpConfirmations(): Promise<PendingMcpConfirmation[]> {
  return invokeOrThrow<PendingMcpConfirmation[]>("list_pending_mcp_confirmations");
}

export function approveMcpConfirmation(operationId: string): Promise<PendingMcpConfirmation> {
  return invokeOrThrow<PendingMcpConfirmation>("approve_mcp_confirmation", { operationId });
}

export function denyMcpConfirmation(operationId: string): Promise<PendingMcpConfirmation> {
  return invokeOrThrow<PendingMcpConfirmation>("deny_mcp_confirmation", { operationId });
}

export function getMcpSettings(): Promise<McpSettings> {
  return invokeOrThrow<McpSettings>("get_mcp_settings");
}

export function listMcpTools(): Promise<McpToolDefinition[]> {
  return invokeOrThrow<McpToolDefinition[]>("list_mcp_tools");
}

export function updateMcpSettings(settings: McpSettings): Promise<McpRuntimeStatus> {
  return invokeOrThrow<McpRuntimeStatus>("update_mcp_settings", { settings });
}

export function getMcpStatus(): Promise<McpRuntimeStatus> {
  return invokeOrThrow<McpRuntimeStatus>("get_mcp_status");
}

export function startMcpHttp(): Promise<McpRuntimeStatus> {
  return invokeOrThrow<McpRuntimeStatus>("start_mcp_http");
}

export function stopMcpHttp(): Promise<McpRuntimeStatus> {
  return invokeOrThrow<McpRuntimeStatus>("stop_mcp_http");
}

export function getMcpClientSnippets(): Promise<McpClientSnippets> {
  return invokeOrThrow<McpClientSnippets>("get_mcp_client_snippets");
}

export function listVersions(
  sourceId: string,
  path: string,
  limit?: number,
  cursor?: string,
): Promise<ListVersionsResult> {
  return invokeOrThrow<ListVersionsResult>("list_versions", { sourceId, path, limit, cursor });
}

export async function readFileVersion(
  sourceId: string,
  path: string,
  version: string,
): Promise<Uint8Array> {
  const data = await invokeOrThrow<number[]>("read_file_version", { sourceId, path, version });
  return new Uint8Array(data);
}

export function deleteFileVersion(
  sourceId: string,
  path: string,
  version: string,
): Promise<DeleteVersionResult> {
  return invokeOrThrow<DeleteVersionResult>("delete_version", { sourceId, path, version });
}
