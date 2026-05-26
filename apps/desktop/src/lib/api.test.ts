import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import {
  addStorage,
  approveMcpConfirmation,
  cancelTransferJob,
  clearMcpAuditEvents,
  completeOnboarding,
  createDirectory,
  deleteFileVersion,
  deletePath,
  denyMcpConfirmation,
  exportMcpAuditBundle,
  exportStorageConfig,
  generateDownloadLink,
  getAppSettings,
  getMcpClientSnippets,
  getMcpSettings,
  getMcpStatus,
  getStorageCapabilities,
  importStorageConfig,
  listActiveMcpSessions,
  listEntries,
  listEntriesRecursive,
  listMcpAuditEvents,
  listMcpTools,
  listPendingMcpConfirmations,
  listStorageSchemas,
  listStorages,
  planTransferEntries,
  listVersions,
  readFile,
  readFileVersion,
  removeStorage,
  skipOnboarding,
  startMcpHttp,
  statEntry,
  stopMcpHttp,
  TauriApiError,
  transferEntries,
  updateMcpSettings,
  updateMcpStoragePolicy,
  updateStorage,
  uploadDroppedFiles,
  verifyStorage,
  writeFile,
} from "./api";
import type { McpSettings, McpStoragePolicy, StorageDraft } from "@/types/storage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

const draft: StorageDraft = {
  name: "Local",
  backend: "local",
  config: { root: "/tmp" },
  enabled: true,
  mcpExposed: true,
  readOnly: false,
};

const policy: McpStoragePolicy = {
  default_access: "read_only",
  allowed_paths: ["docs"],
  denied_paths: ["private"],
  confirmation_rules: {
    require_for_write: true,
    require_for_overwrite: true,
    require_for_delete: true,
    require_for_version_delete: true,
    require_for_presign: true,
    require_for_cross_storage_copy: true,
  },
};

const settings: McpSettings = {
  enabled: true,
  transport: "http",
  bindAddress: "127.0.0.1",
  port: 7331,
  enabledTools: ["list_dir"],
};

const auditEvent = {
  id: "audit-1",
  timestamp: "2026-01-01T00:00:00Z",
  actor_type: "mcp",
  mcp_client_id: null,
  session_id: null,
  storage_id: "s1",
  storage_name: "Local",
  backend: "local",
  tool_name: "read_file",
  operation: "read",
  path: "/Local/file.txt",
  version_id: null,
  decision: "allowed",
  confirmation_id: null,
  duration_ms: 3,
  bytes_read: null,
  bytes_written: null,
  error_code: null,
};

describe("api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  it.each([
    ["listEntries", () => listEntries("s1", "/a"), "list_entries", { sourceId: "s1", path: "/a" }],
    ["listEntriesRecursive", () => listEntriesRecursive("s1", "/a"), "list_entries_recursive", { sourceId: "s1", path: "/a" }],
    ["statEntry", () => statEntry("s1", "/a"), "stat_entry", { sourceId: "s1", path: "/a" }],
    ["createDirectory", () => createDirectory("s1", "/new"), "create_directory", { sourceId: "s1", path: "/new" }],
    ["uploadDroppedFiles", () => uploadDroppedFiles("s1", ["/tmp/a.txt"], "/dst"), "upload_dropped_files", { sourceId: "s1", paths: ["/tmp/a.txt"], targetDir: "/dst" }],
    ["deletePath", () => deletePath("s1", "/old"), "delete_path", { sourceId: "s1", path: "/old" }],
    ["cancelTransferJob", () => cancelTransferJob("transfer-1"), "cancel_transfer_job", { jobId: "transfer-1" }],
    ["listStorages", () => listStorages(), "list_storages", undefined],
    ["addStorage", () => addStorage(draft), "add_storage", { storage: draft }],
    ["updateStorage", () => updateStorage("s1", draft), "update_storage", { storageId: "s1", storage: draft }],
    ["removeStorage", () => removeStorage("s1"), "remove_storage", { storageId: "s1" }],
    ["updateMcpStoragePolicy", () => updateMcpStoragePolicy("s1", policy), "update_mcp_storage_policy", { storageId: "s1", policy }],
    ["verifyStorage", () => verifyStorage(draft), "verify_storage", { storage: draft }],
    ["listStorageSchemas", () => listStorageSchemas(), "list_storage_schemas", undefined],
    ["getStorageCapabilities", () => getStorageCapabilities("s1"), "get_storage_capabilities", { storageId: "s1" }],
    ["generateDownloadLink", () => generateDownloadLink("s1", "/file.txt", 60), "generate_download_link", { sourceId: "s1", path: "/file.txt", expiresSeconds: 60 }],
    ["getAppSettings", () => getAppSettings(), "get_app_settings", undefined],
    ["completeOnboarding", () => completeOnboarding(), "complete_onboarding", undefined],
    ["skipOnboarding", () => skipOnboarding(), "skip_onboarding", undefined],
    ["listMcpAuditEvents", () => listMcpAuditEvents(25), "list_mcp_audit_events", { limit: 25 }],
    ["clearMcpAuditEvents", () => clearMcpAuditEvents(), "clear_mcp_audit_events", undefined],
    ["exportMcpAuditBundle", () => exportMcpAuditBundle([auditEvent]), "export_mcp_audit_bundle", { request: { events: [auditEvent] } }],
    ["listPendingMcpConfirmations", () => listPendingMcpConfirmations(), "list_pending_mcp_confirmations", undefined],
    ["listActiveMcpSessions", () => listActiveMcpSessions(), "list_active_mcp_sessions", undefined],
    ["approveMcpConfirmation", () => approveMcpConfirmation("op-1"), "approve_mcp_confirmation", { operationId: "op-1" }],
    ["denyMcpConfirmation", () => denyMcpConfirmation("op-1"), "deny_mcp_confirmation", { operationId: "op-1" }],
    ["getMcpSettings", () => getMcpSettings(), "get_mcp_settings", undefined],
    ["listMcpTools", () => listMcpTools(), "list_mcp_tools", undefined],
    ["updateMcpSettings", () => updateMcpSettings(settings), "update_mcp_settings", { settings }],
    ["getMcpStatus", () => getMcpStatus(), "get_mcp_status", undefined],
    ["startMcpHttp", () => startMcpHttp(), "start_mcp_http", undefined],
    ["stopMcpHttp", () => stopMcpHttp(), "stop_mcp_http", undefined],
    ["getMcpClientSnippets", () => getMcpClientSnippets(), "get_mcp_client_snippets", undefined],
    ["listVersions", () => listVersions("s1", "/file.txt", 20, "cursor-1"), "list_versions", { sourceId: "s1", path: "/file.txt", limit: 20, cursor: "cursor-1" }],
    ["deleteFileVersion", () => deleteFileVersion("s1", "/file.txt", "v1"), "delete_version", { sourceId: "s1", path: "/file.txt", version: "v1" }],
  ])("calls %s with the expected Tauri command", async (_label, call, command, payload) => {
    invokeMock.mockResolvedValue("ok");

    await call();

    if (payload === undefined) {
      expect(invokeMock).toHaveBeenCalledWith(command);
    } else {
      expect(invokeMock).toHaveBeenCalledWith(command, payload);
    }
  });

  it("serializes binary file reads and writes", async () => {
    invokeMock.mockResolvedValueOnce([104, 105]).mockResolvedValueOnce(undefined);

    await expect(readFile("s1", "/hi.txt")).resolves.toEqual(new Uint8Array([104, 105]));
    await writeFile("s1", "/hi.txt", new Uint8Array([1, 2, 3]));
    await writeFile("s1", "/meta.txt", new Uint8Array([4]), { project: "alpha" });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "read_file", { sourceId: "s1", path: "/hi.txt" });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "write_file", {
      sourceId: "s1",
      path: "/hi.txt",
      data: [1, 2, 3],
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "write_file", {
      sourceId: "s1",
      path: "/meta.txt",
      data: [4],
      userMetadata: { project: "alpha" },
    });
  });

  it("serializes transfer, import, export, and version file reads", async () => {
    invokeMock
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ imported: 1 })
      .mockResolvedValueOnce({ json: "[]" })
      .mockResolvedValueOnce([1, 2]);

    await transferEntries("from", "to", ["/a.txt"], "/target", "copy", "overwrite");
    await importStorageConfig({ json: "[]", mode: "merge", onConflict: "rename" });
    await exportStorageConfig(false);
    await expect(readFileVersion("s1", "/file.txt", "v1")).resolves.toEqual(new Uint8Array([1, 2]));

    expect(invokeMock).toHaveBeenNthCalledWith(1, "transfer_entries", {
      fromSourceId: "from",
      toSourceId: "to",
      paths: ["/a.txt"],
      targetDir: "/target",
      operation: "copy",
      conflictPolicy: "overwrite",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "import_storage_config", {
      request: { json: "[]", mode: "merge", onConflict: "rename" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "export_storage_config", {
      includeSecrets: false,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "read_file_version", {
      sourceId: "s1",
      path: "/file.txt",
      version: "v1",
    });
  });

  it("serializes transfer dry-run plans", async () => {
    invokeMock.mockResolvedValue({ entries: [], summary: { totalItems: 0, totalBytes: 0 } });

    await planTransferEntries("from", "to", ["/a.txt"], "/target", "copy", "rename");

    expect(invokeMock).toHaveBeenCalledWith("plan_transfer_entries", {
      fromSourceId: "from",
      toSourceId: "to",
      paths: ["/a.txt"],
      targetDir: "/target",
      operation: "copy",
      conflictPolicy: "rename",
    });
  });

  it("serializes move transfers with optional job ids", async () => {
    invokeMock.mockResolvedValue(undefined);

    await transferEntries("from", "to", ["/a.txt"], "/target", "move", "fail", "transfer-1");

    expect(invokeMock).toHaveBeenCalledWith("transfer_entries", {
      fromSourceId: "from",
      toSourceId: "to",
      paths: ["/a.txt"],
      targetDir: "/target",
      operation: "move",
      conflictPolicy: "fail",
      jobId: "transfer-1",
    });
  });

  it("defaults download link expiry and audit limit", async () => {
    invokeMock.mockResolvedValue("ok");

    await generateDownloadLink("s1", "/file.txt");
    await listMcpAuditEvents();

    expect(invokeMock).toHaveBeenNthCalledWith(1, "generate_download_link", {
      sourceId: "s1",
      path: "/file.txt",
      expiresSeconds: 900,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "list_mcp_audit_events", { limit: 200 });
  });

  it("maps structured Tauri errors into TauriApiError", async () => {
    invokeMock.mockRejectedValue({ code: "ERR_MCP_POLICY_DENIED", message: "Denied" });

    await expect(listEntries("s1", "/private")).rejects.toMatchObject({
      name: "TauriApiError",
      code: "ERR_MCP_POLICY_DENIED",
      message: "Denied",
    });
  });

  it("maps string and Error failures into UNKNOWN TauriApiError", async () => {
    invokeMock.mockRejectedValueOnce("plain failure").mockRejectedValueOnce(new Error("boom"));

    await expect(getMcpStatus()).rejects.toEqual(new TauriApiError("plain failure"));
    await expect(getMcpStatus()).rejects.toEqual(new TauriApiError("boom"));
  });
});
