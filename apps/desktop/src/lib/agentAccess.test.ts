import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getMcpStatus, updateMcpSettings } from "@/lib/api";
import { prepareWorkspaceAgentAccess, workspaceAgentSettings } from "./agentAccess";
import type { McpRuntimeStatus } from "@/types/storage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@/lib/api", () => ({
  getMcpStatus: vi.fn(),
  updateMcpSettings: vi.fn(),
}));

function status(overrides: Partial<McpRuntimeStatus["settings"]> = {}): McpRuntimeStatus {
  return {
    settings: {
      enabled: false,
      transport: "http",
      bindAddress: "0.0.0.0",
      port: 7331,
      enabledTools: [],
      securityBaselineVersion: 2,
      authTokenConfigured: false,
      ...overrides,
    },
    runningHttp: false,
    endpoint: null,
    endpointDisplay: "",
    authTokenConfigured: false,
  };
}

describe("workspace agent access", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getMcpStatus).mockResolvedValue(status());
    vi.mocked(updateMcpSettings).mockResolvedValue(status({ enabled: true, transport: "stdio" }));
  });

  it("uses local stdio and only read tools for first-time read-only setup", () => {
    const update = workspaceAgentSettings(status(), "read_only");
    expect(update.enabled).toBe(true);
    expect(update.transport).toBe("stdio");
    expect(update.bindAddress).toBe("127.0.0.1");
    expect(update.enabledTools).toEqual([
      "list_dir",
      "list_versions",
      "read_file",
      "read_file_version",
      "search_paths",
      "stat_path",
    ]);
  });

  it("adds only the minimum workspace writes for a read-write workspace", () => {
    const update = workspaceAgentSettings(status(), "read_write");
    expect(update.enabledTools).toEqual(
      expect.arrayContaining(["list_dir", "read_file", "mkdir", "write_file"]),
    );
    expect(update.enabledTools).not.toContain("copy_path");
    expect(update.enabledTools).not.toContain("delete_path");
    expect(update.enabledTools).not.toContain("move_path");
    expect(update.enabledTools).not.toContain("generate_download_link");
  });

  it("preserves an already-active user's transport and explicit tools", () => {
    const update = workspaceAgentSettings(
      status({
        enabled: true,
        transport: "http",
        bindAddress: "127.0.0.1",
        enabledTools: ["delete_path", "read_file"],
      }),
      "read_write",
    );
    expect(update.transport).toBe("http");
    expect(update.enabledTools).toContain("delete_path");
    expect(update.enabledTools).toContain("write_file");
    expect(update.enabledTools.filter((tool) => tool === "read_file")).toHaveLength(1);
  });

  it("validates policy before changing MCP settings and revalidates before exposure", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({
        workspaceId: "workspace-id",
        storageId: "storage-id",
        accessProfile: "read_write",
        mcpExposed: false,
        changed: false,
      })
      .mockResolvedValueOnce({
        workspaceId: "workspace-id",
        storageId: "storage-id",
        accessProfile: "read_write",
        mcpExposed: true,
        changed: true,
      });

    await expect(prepareWorkspaceAgentAccess("workspace-id", "read_write")).resolves.toMatchObject({
      mcpExposed: true,
      changed: true,
    });

    expect(vi.mocked(invoke).mock.calls[0]).toEqual([
      "check_workspace_agent_access",
      { workspaceId: "workspace-id" },
    ]);
    expect(vi.mocked(invoke).mock.calls[1]).toEqual([
      "prepare_workspace_agent_access",
      { workspaceId: "workspace-id" },
    ]);
    expect(vi.mocked(invoke).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(updateMcpSettings).mock.invocationCallOrder[0],
    );
    expect(vi.mocked(updateMcpSettings).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(invoke).mock.invocationCallOrder[1],
    );
  });

  it("does not touch MCP runtime settings when policy preflight rejects the workspace", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({ code: "ERR_CONFIRMATION_REQUIRED" });

    await expect(prepareWorkspaceAgentAccess("workspace-id", "read_write")).rejects.toThrow(
      /broader MCP grants/i,
    );
    expect(getMcpStatus).not.toHaveBeenCalled();
    expect(updateMcpSettings).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
