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

export function workspaceStorageIssue(storage: StorageConfig): string | null {
  if (!storage.enabled) return "This storage is disabled.";
  if (storage.readOnly) return "This storage is read-only. Agent Workspaces need a writable storage.";

  if (storage.type === "local-fs" || storage.backend === "local" || storage.backend === "fs") {
    const root = storageConfigString(storage, ["rootPath", "root", "path"]);
    if (!root) return "This Local Filesystem storage has no configured root folder.";
    if (root === "$HOME" || root.startsWith("$HOME/")) {
      return "This storage uses $HOME shell syntax. Edit the storage and use an absolute folder path instead.";
    }
    if (root === "~" || root.startsWith("~/")) {
      return "This storage uses a ~ path. Edit the storage and use an absolute folder path before creating an Agent Workspace.";
    }
    if (!isAbsoluteLocalRoot(root)) {
      return "This Local Filesystem storage root is not absolute. Edit and validate the storage first.";
    }
  }

  return null;
}
