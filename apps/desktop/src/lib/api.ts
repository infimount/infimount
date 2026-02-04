import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  modified_at: string | null;
}

import type { Source, SourceKind } from "../types/source";

export interface ApiError {
  code: string;
  message: string;
}

export class TauriApiError extends Error {
  code: string;

  constructor(message: string, code: string = "UNKNOWN") {
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
  kind: SourceKind;
  fields: StorageFieldSchema[];
}

export type TransferOperation = "copy" | "move";
export type TransferConflictPolicy = "fail" | "overwrite" | "skip";

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
    return new Uint8Array(data as number[]);
  } catch (error) {
    return handleError(error);
  }
}

export async function writeFile(
  sourceId: string,
  path: string,
  data: Uint8Array
): Promise<void> {
  try {
    const dataArray = Array.from(data);
    return await tauriInvoke("write_file", { sourceId, path, data: dataArray });
  } catch (error) {
    return handleError(error);
  }
}

export async function uploadDroppedFiles(
  sourceId: string,
  paths: string[],
  targetDir: string
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

export async function listSources(): Promise<Source[]> {
  try {
    return await tauriInvoke<Source[]>("list_sources");
  } catch (error) {
    return handleError(error);
  }
}

export async function addSource(source: Source): Promise<void> {
  try {
    return await tauriInvoke("add_source", { source });
  } catch (error) {
    return handleError(error);
  }
}

export async function removeSource(sourceId: string): Promise<void> {
  try {
    return await tauriInvoke("remove_source", { sourceId });
  } catch (error) {
    return handleError(error);
  }
}

export async function updateSource(source: Source): Promise<void> {
  try {
    return await tauriInvoke("update_source", { source });
  } catch (error) {
    return handleError(error);
  }
}

export async function replaceSources(sources: Source[]): Promise<void> {
  try {
    return await tauriInvoke("replace_sources", { sources });
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
