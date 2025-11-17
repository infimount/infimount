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
  message: string;
}

export class TauriApiError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TauriApiError";
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

async function handleError(error: any): Promise<never> {
  const message = typeof error === "string" ? error : error.message || String(error);
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
