import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettings,
  McpToolDefinition,
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
export type TransferConflictPolicy = "fail" | "overwrite" | "skip";

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

export async function listEntries(sourceId: string, path: string): Promise<Entry[]> {
  try {
    return await tauriInvoke<Entry[]>("list_entries", { sourceId, path });
  } catch (error) {
    return handleError(error);
  }
}

export async function statEntry(sourceId: string, path: string): Promise<Entry> {
  try {
    return await tauriInvoke<Entry>("stat_entry", { sourceId, path });
  } catch (error) {
    return handleError(error);
  }
}

export async function readFile(sourceId: string, path: string): Promise<Uint8Array> {
  try {
    const data = await tauriInvoke<number[]>("read_file", { sourceId, path });
    return new Uint8Array(data);
  } catch (error) {
    return handleError(error);
  }
}

export async function writeFile(
  sourceId: string,
  path: string,
  data: Uint8Array,
): Promise<void> {
  try {
    return await tauriInvoke("write_file", { sourceId, path, data: Array.from(data) });
  } catch (error) {
    return handleError(error);
  }
}

export async function createDirectory(sourceId: string, path: string): Promise<void> {
  try {
    return await tauriInvoke("create_directory", { sourceId, path });
  } catch (error) {
    return handleError(error);
  }
}

export async function uploadDroppedFiles(
  sourceId: string,
  paths: string[],
  targetDir: string,
): Promise<void> {
  try {
    return await tauriInvoke("upload_dropped_files", { sourceId, paths, targetDir });
  } catch (error) {
    return handleError(error);
  }
}

export async function deletePath(sourceId: string, path: string): Promise<void> {
  try {
    return await tauriInvoke("delete_path", { sourceId, path });
  } catch (error) {
    return handleError(error);
  }
}

export async function transferEntries(
  fromSourceId: string,
  toSourceId: string,
  paths: string[],
  targetDir: string,
  operation: TransferOperation,
  conflictPolicy: TransferConflictPolicy,
): Promise<void> {
  try {
    return await tauriInvoke("transfer_entries", {
      fromSourceId,
      toSourceId,
      paths,
      targetDir,
      operation,
      conflictPolicy,
    });
  } catch (error) {
    return handleError(error);
  }
}

export async function listStorages(): Promise<StorageConfig[]> {
  try {
    return await tauriInvoke<StorageConfig[]>("list_storages");
  } catch (error) {
    return handleError(error);
  }
}

export async function addStorage(storage: StorageDraft): Promise<StorageConfig> {
  try {
    return await tauriInvoke<StorageConfig>("add_storage", { storage });
  } catch (error) {
    return handleError(error);
  }
}

export async function updateStorage(
  storageId: string,
  storage: StorageDraft,
): Promise<StorageConfig> {
  try {
    return await tauriInvoke<StorageConfig>("update_storage", { storageId, storage });
  } catch (error) {
    return handleError(error);
  }
}

export async function removeStorage(storageId: string): Promise<void> {
  try {
    return await tauriInvoke("remove_storage", { storageId });
  } catch (error) {
    return handleError(error);
  }
}

export async function verifyStorage(storage: StorageDraft): Promise<StorageValidationResult> {
  try {
    return await tauriInvoke<StorageValidationResult>("verify_storage", { storage });
  } catch (error) {
    return handleError(error);
  }
}

export async function importStorageConfig(
  request: ImportStoragesRequest,
): Promise<ImportStoragesResult> {
  try {
    return await tauriInvoke<ImportStoragesResult>("import_storage_config", {
      request: {
        json: request.json,
        mode: request.mode,
        onConflict: request.onConflict,
      },
    });
  } catch (error) {
    return handleError(error);
  }
}

export async function exportStorageConfig(
  includeSecrets: boolean,
): Promise<ExportStoragesResult> {
  try {
    return await tauriInvoke<ExportStoragesResult>("export_storage_config", {
      includeSecrets,
    });
  } catch (error) {
    return handleError(error);
  }
}

export async function listStorageSchemas(): Promise<StorageKindSchema[]> {
  try {
    return await tauriInvoke<StorageKindSchema[]>("list_storage_schemas");
  } catch (error) {
    return handleError(error);
  }
}

export async function getMcpSettings(): Promise<McpSettings> {
  try {
    return await tauriInvoke<McpSettings>("get_mcp_settings");
  } catch (error) {
    return handleError(error);
  }
}

export async function listMcpTools(): Promise<McpToolDefinition[]> {
  try {
    return await tauriInvoke<McpToolDefinition[]>("list_mcp_tools");
  } catch (error) {
    return handleError(error);
  }
}

export async function updateMcpSettings(settings: McpSettings): Promise<McpRuntimeStatus> {
  try {
    return await tauriInvoke<McpRuntimeStatus>("update_mcp_settings", { settings });
  } catch (error) {
    return handleError(error);
  }
}

export async function getMcpStatus(): Promise<McpRuntimeStatus> {
  try {
    return await tauriInvoke<McpRuntimeStatus>("get_mcp_status");
  } catch (error) {
    return handleError(error);
  }
}

export async function startMcpHttp(): Promise<McpRuntimeStatus> {
  try {
    return await tauriInvoke<McpRuntimeStatus>("start_mcp_http");
  } catch (error) {
    return handleError(error);
  }
}

export async function stopMcpHttp(): Promise<McpRuntimeStatus> {
  try {
    return await tauriInvoke<McpRuntimeStatus>("stop_mcp_http");
  } catch (error) {
    return handleError(error);
  }
}

export async function getMcpClientSnippets(): Promise<McpClientSnippets> {
  try {
    return await tauriInvoke<McpClientSnippets>("get_mcp_client_snippets");
  } catch (error) {
    return handleError(error);
  }
}
