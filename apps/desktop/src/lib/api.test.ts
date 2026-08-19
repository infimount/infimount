import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import {
  addStorage,
  approveMcpConfirmation,
  cancelTransferJob,
  clearMcpAuditEvents,
  completeOnboarding,
  createDirectory,
  createWorkspaceCheckpointCommand,
  listWorkspaceCheckpoints,
  restoreWorkspaceCheckpointCommand,
  deleteFileVersion,
  deletePath,
  deleteWorkspaceWithFiles,
  denyMcpConfirmation,
  downloadFileToDownloads,
  downloadFileVersionToDownloads,
  exportMcpAuditBundle,
  exportShareableConfig,
  generateDownloadLink,
  getAppSettings,
  getStartupHealth,
  getMcpClientSnippets,
  getMcpStatus,
  getStorageCapabilities,
  applyStorageImport,
  listActiveMcpSessions,
  listEntries,
  listEntriesRecursive,
  listMcpAuditEvents,
  listMcpClientAdapters,
  listMcpTools,
  listPendingMcpConfirmations,
  listStorageSchemas,
  listStorages,
  planTransferEntries,
  previewStorageImport,
  previewMcpClientInstall,
  applyMcpClientInstall,
  rollbackMcpClientInstall,
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
  uploadFileStreaming,
  verifyStorage,
  writeFile,
} from "./api";
import type { McpSettingsUpdate, McpStoragePolicy, StorageDraft } from "@/types/storage";

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
  version: 2,
  default_access: "read_only",
  rules: [],
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

const settingsUpdate: McpSettingsUpdate = {
  enabled: true,
  transport: "http",
  bindAddress: "127.0.0.1",
  port: 7331,
  enabledTools: ["list_dir"],
  authTokenMutation: { action: "set", value: "test-token" },
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
    ["updateStorage", () => updateStorage("s1", draft), "update_storage", { storageId: "s1", storage: draft, confirmWorkspaceCredentialChange: false }],
    ["removeStorage", () => removeStorage("s1"), "remove_storage", { storageId: "s1" }],
    ["updateMcpStoragePolicy", () => updateMcpStoragePolicy("s1", policy), "update_mcp_storage_policy", { storageId: "s1", policy }],
    ["verifyStorage", () => verifyStorage(draft), "verify_storage", { storage: draft }],
    ["listStorageSchemas", () => listStorageSchemas(), "list_storage_schemas", undefined],
    ["getStorageCapabilities", () => getStorageCapabilities("s1"), "get_storage_capabilities", { storageId: "s1" }],
    ["generateDownloadLink", () => generateDownloadLink("s1", "/file.txt", 60), "generate_download_link", { sourceId: "s1", path: "/file.txt", expiresSeconds: 60 }],
    ["getAppSettings", () => getAppSettings(), "get_app_settings", undefined],
    ["getStartupHealth", () => getStartupHealth(), "get_startup_health", undefined],
    ["completeOnboarding", () => completeOnboarding(), "complete_onboarding", undefined],
    ["skipOnboarding", () => skipOnboarding(), "skip_onboarding", undefined],
    ["listMcpAuditEvents", () => listMcpAuditEvents(25), "list_mcp_audit_events", { limit: 25 }],
    ["clearMcpAuditEvents", () => clearMcpAuditEvents(), "clear_mcp_audit_events", undefined],
    ["exportMcpAuditBundle", () => exportMcpAuditBundle([auditEvent]), "export_mcp_audit_bundle", { request: { events: [auditEvent] } }],
    ["listPendingMcpConfirmations", () => listPendingMcpConfirmations(), "list_pending_mcp_confirmations", undefined],
    ["listActiveMcpSessions", () => listActiveMcpSessions(), "list_active_mcp_sessions", undefined],
    ["approveMcpConfirmation", () => approveMcpConfirmation("op-1"), "approve_mcp_confirmation", { operationId: "op-1" }],
    ["denyMcpConfirmation", () => denyMcpConfirmation("op-1"), "deny_mcp_confirmation", { operationId: "op-1" }],
    ["listMcpTools", () => listMcpTools(), "list_mcp_tools", undefined],
    ["updateMcpSettings", () => updateMcpSettings(settingsUpdate), "update_mcp_settings_with_auth", { update: settingsUpdate }],
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

  it("serializes transfer, safe import, export, and version file reads", async () => {
    invokeMock
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ previewId: "preview-1" })
      .mockResolvedValueOnce({ applied: 1 })
      .mockResolvedValueOnce({ json: "[]" })
      .mockResolvedValueOnce([1, 2]);

    await transferEntries("from", "to", ["/a.txt"], "/target", "copy", "overwrite");
    await previewStorageImport({ json: "[]", mode: "merge", onConflict: "rename" });
    await applyStorageImport({ previewId: "preview-1", confirmed: false });
    await exportShareableConfig();
    await expect(readFileVersion("s1", "/file.txt", "v1")).resolves.toEqual(new Uint8Array([1, 2]));

    expect(invokeMock).toHaveBeenNthCalledWith(1, "transfer_entries", {
      fromSourceId: "from",
      toSourceId: "to",
      paths: ["/a.txt"],
      targetDir: "/target",
      operation: "copy",
      conflictPolicy: "overwrite",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "preview_storage_import_cmd", {
      request: { json: "[]", mode: "merge", onConflict: "rename" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "apply_storage_import_cmd", {
      request: { previewId: "preview-1", confirmed: false },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "export_shareable_config");
    expect(invokeMock).toHaveBeenNthCalledWith(5, "read_file_version", {
      sourceId: "s1",
      path: "/file.txt",
      version: "v1",
    });
  });

  it("maps MCP client adapter preview, apply, and rollback commands", async () => {
    invokeMock.mockResolvedValue(undefined);
    await listMcpClientAdapters();
    await previewMcpClientInstall("cursor", "/project/.cursor/mcp.json");
    await applyMcpClientInstall("preview-1", false);
    await rollbackMcpClientInstall("rollback-1");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "list_mcp_client_adapters");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "preview_mcp_client_install", {
      input: { kind: "cursor", targetPath: "/project/.cursor/mcp.json" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "apply_mcp_client_install", {
      input: { previewId: "preview-1", confirmExecution: false },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "rollback_mcp_client_install", {
      rollbackId: "rollback-1",
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

  it("routes desktop downloads through the native streaming command", async () => {
    invokeMock.mockResolvedValue({ fileName: "report.txt", bytes: 12 });
    await expect(downloadFileToDownloads("s1", "/report.txt")).resolves.toEqual({
      fileName: "report.txt",
      bytes: 12,
    });
    expect(invokeMock).toHaveBeenCalledWith("download_file_to_downloads", {
      sourceId: "s1",
      path: "/report.txt",
    });
  });

  it("routes version downloads through native streaming without IPC bytes", async () => {
    invokeMock.mockResolvedValue({ fileName: "report (2).txt", bytes: 12 });
    await expect(downloadFileVersionToDownloads("s1", "/report.txt", "v2")).resolves.toEqual({
      fileName: "report (2).txt",
      bytes: 12,
    });
    expect(invokeMock).toHaveBeenCalledWith("download_file_version_to_downloads", {
      sourceId: "s1",
      path: "/report.txt",
      version: "v2",
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

  it("streams browser uploads through a bounded staging lifecycle", async () => {
    invokeMock
      .mockResolvedValueOnce("upload-1")
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined);

    const bytes = new TextEncoder().encode("hello");
    await uploadFileStreaming("s1", "/file.txt", {
      size: bytes.byteLength,
      arrayBuffer: async () => bytes.buffer,
      slice: (start = 0, end = bytes.byteLength) => ({
        arrayBuffer: async () => bytes.slice(start, end).buffer,
      }) as Blob,
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "begin_file_upload");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "append_file_upload_chunk", {
      uploadId: "upload-1",
      data: [104, 101, 108, 108, 111],
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "finish_file_upload", {
      uploadId: "upload-1",
      sourceId: "s1",
      targetPath: "/file.txt",
    });
  });

  it("cancels an active native finalization immediately", async () => {
    let rejectFinish!: (error: Error) => void;
    invokeMock.mockImplementation((command: string) => {
      if (command === "begin_file_upload") return Promise.resolve("upload-active");
      if (command === "append_file_upload_chunk") return Promise.resolve(undefined);
      if (command === "finish_file_upload") {
        return new Promise((_, reject) => {
          rejectFinish = reject;
        });
      }
      return Promise.resolve(undefined);
    });
    const bytes = new TextEncoder().encode("hello");
    const controller = new AbortController();
    const upload = uploadFileStreaming(
      "s1",
      "/file.txt",
      {
        size: bytes.byteLength,
        arrayBuffer: async () => bytes.buffer,
        slice: () => ({ arrayBuffer: async () => bytes.buffer }) as Blob,
      },
      { signal: controller.signal },
    );
    while (!rejectFinish) await Promise.resolve();
    controller.abort();
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledWith("cancel_file_upload", { uploadId: "upload-active" });
    rejectFinish(new Error("upload cancelled"));
    await expect(upload).rejects.toThrow("upload cancelled");
  });

  it("cleans up a cancelled browser upload", async () => {
    invokeMock.mockResolvedValueOnce("upload-2").mockResolvedValueOnce(undefined);
    const bytes = new TextEncoder().encode("hello");
    await expect(
      uploadFileStreaming("s1", "/file.txt", {
        size: bytes.byteLength,
        arrayBuffer: async () => bytes.buffer,
        slice: () => ({ arrayBuffer: async () => bytes.buffer }) as Blob,
      }, {
        isCancelled: () => true,
      }),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(invokeMock).toHaveBeenLastCalledWith("cancel_file_upload", { uploadId: "upload-2" });
  });

  it("routes workspace checkpoints through authoritative backend commands", async () => {
    invokeMock.mockResolvedValue([]);
    await listWorkspaceCheckpoints("workspace-1");
    expect(invokeMock).toHaveBeenLastCalledWith("list_workspace_checkpoints", {
      workspaceId: "workspace-1",
    });

    invokeMock.mockResolvedValue({ id: "checkpoint-1" });
    await createWorkspaceCheckpointCommand("workspace-1", "Before change");
    expect(invokeMock).toHaveBeenLastCalledWith("create_workspace_checkpoint", {
      request: { workspaceId: "workspace-1", label: "Before change" },
    });

    invokeMock.mockResolvedValue(undefined);
    await restoreWorkspaceCheckpointCommand("workspace-1", "checkpoint-1", true);
    expect(invokeMock).toHaveBeenLastCalledWith("restore_workspace_checkpoint", {
      request: { workspaceId: "workspace-1", checkpointId: "checkpoint-1", confirmOverwrite: true },
    });
  });

  it("requires an explicit backend confirmation flag for workspace file deletion", async () => {
    invokeMock.mockResolvedValue(undefined);
    await deleteWorkspaceWithFiles("workspace-1", true);
    expect(invokeMock).toHaveBeenCalledWith("delete_workspace_with_files", {
      request: { id: "workspace-1", confirmDeleteFiles: true },
    });
  });

  it("rejects non-slice upload sources before reading or opening a session", async () => {
    const arrayBuffer = vi.fn(async () => new ArrayBuffer(16));
    await expect(uploadFileStreaming("s1", "/file.txt", { arrayBuffer }))
      .rejects.toThrow("bounded chunk reads");
    expect(arrayBuffer).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
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
