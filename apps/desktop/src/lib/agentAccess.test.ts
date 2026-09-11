import { describe, expect, it } from "vitest";

import { workspaceAgentSettings } from "./agentAccess";
import type { McpRuntimeStatus } from "@/types/storage";

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

describe("workspaceAgentSettings", () => {
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

  it("adds non-destructive workspace writes for a read-write workspace", () => {
    const update = workspaceAgentSettings(status(), "read_write");
    expect(update.enabledTools).toEqual(
      expect.arrayContaining(["list_dir", "read_file", "mkdir", "write_file", "copy_path"]),
    );
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
});
