import { invoke } from "@tauri-apps/api/core";

import type { StorageConfig } from "@/types/storage";

function storageConfigString(storage: StorageConfig, keys: string[]): string | null {
  for (const key of keys) {
    const value = storage.config[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function isAbsoluteLocalRoot(value: string): boolean {
  if (value.startsWith("/")) return true;
  if (/^[A-Za-z]:[\\/]/.test(value)) return true;
  if (value.startsWith("\\\\")) return true;
  return false;
}

function isSupportedHomeAlias(value: string): boolean {
  return value === "~" || value.startsWith("~/") || value.startsWith("~\\");
}

export interface WorkspaceStorageBindingResult {
  storageId: string;
  normalized: boolean;
}

export async function prepareWorkspaceStorageBinding(
  storageId: string,
): Promise<WorkspaceStorageBindingResult> {
  try {
    return await invoke<WorkspaceStorageBindingResult>("prepare_workspace_storage_binding", {
      storageId,
    });
  } catch (error) {
    const message =
      typeof error === "object" && error !== null && "message" in error
        ? String((error as { message: unknown }).message)
        : "";
    throw new Error(message || "The storage could not be prepared for an Agent Workspace.", {
      cause: error,
    });
  }
}

export function workspaceStorageIssue(storage: StorageConfig): string | null {
  if (!storage.enabled) return "This storage is disabled.";
  if (storage.readOnly) return "This storage is read-only. Agent Workspaces need a writable storage.";

  if (storage.type === "local-fs" || storage.backend === "local" || storage.backend === "fs") {
    // Keep this precedence aligned with the backend's local_root_from_config helper.
    const root = storageConfigString(storage, ["root", "rootPath", "path"]);
    if (!root) return "This Local Filesystem storage has no configured root folder.";
    if (root === "$HOME" || root.startsWith("$HOME/") || root.startsWith("${HOME}")) {
      return "This storage uses shell variable syntax. Edit the storage and use either ~ or an absolute folder path.";
    }
    // Legacy ~ roots are normalized to the concrete home directory once, just
    // before the first workspace binds the storage namespace.
    if (isSupportedHomeAlias(root)) return null;
    if (!isAbsoluteLocalRoot(root)) {
      return "This Local Filesystem storage root is not absolute. Edit and validate the storage first.";
    }
  }

  return null;
}
