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
    // The Local Filesystem operator already expands ~ and ~/... to the current
    // user's home directory. Workspace namespace binding supports the same alias.
    if (isSupportedHomeAlias(root)) return null;
    if (!isAbsoluteLocalRoot(root)) {
      return "This Local Filesystem storage root is not absolute. Edit and validate the storage first.";
    }
  }

  return null;
}
