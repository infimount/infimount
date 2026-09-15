import { describe, expect, it } from "vitest";

import { summarizeAgentAccess } from "./agentAccessStatus";
import type { WorkspaceRecord } from "@/lib/api";
import type { McpRuntimeStatus, StorageConfig } from "@/types/storage";

const workspace: WorkspaceRecord = {
  id: "workspace-1",
  storageId: "storage-1",
  name: "Workspace",
  rootPath: "/agent-workspaces/workspace",
  templateId: "custom",
  accessProfile: "read_only",
  policyRuleId: "workspace:workspace-1",
  createdAt: "2026-09-15T00:00:00Z",
  updatedAt: "2026-09-15T00:00:00Z",
  memoryFiles: [],
  checkpointIds: [],
};

const storage = {
  id: "storage-1",
  name: "Storage",
  backend: "local",
  type: "local-fs",
  config: {},
  enabled: true,
  mcpExposed: true,
  readOnly: false,
  connected: true,
  createdAt: "2026-09-15T00:00:00Z",
  updatedAt: "2026-09-15T00:00:00Z",
  mcpPolicy: {
    version: 2,
    default_access: "none",
    rules: [
      {
        id: "workspace:workspace-1",
        prefix: "/agent-workspaces/workspace",
        access: "read_only",
        source: { kind: "workspace", workspace_id: "workspace-1" },
      },
    ],
    denied_paths: [],
    confirmation_rules: {
      require_for_write: true,
      require_for_overwrite: true,
      require_for_delete: true,
      require_for_version_delete: true,
      require_for_presign: true,
      require_for_cross_storage_copy: true,
    },
  },
} satisfies StorageConfig;

function status(
  transport: "stdio" | "http",
  enabled: boolean,
  runningHttp = false,
): McpRuntimeStatus {
  return {
    settings: {
      enabled,
      transport,
      bindAddress: "127.0.0.1",
      port: 7331,
      enabledTools: ["list_dir", "stat_path", "read_file", "search_paths"],
      securityBaselineVersion: 2,
      authTokenConfigured: false,
    },
    runningHttp,
    endpoint: runningHttp ? "http://127.0.0.1:7331/mcp" : null,
    endpointDisplay: runningHttp ? "http://127.0.0.1:7331/mcp" : "Starts on 127.0.0.1:7331/mcp",
    authTokenConfigured: false,
  };
}

describe("summarizeAgentAccess", () => {
  it("reports no workspace as not configured", () => {
    expect(summarizeAgentAccess(status("stdio", false), [], [storage]).state).toBe("not_configured");
  });

  it("reports disabled stdio explicitly", () => {
    expect(summarizeAgentAccess(status("stdio", false), [workspace], [storage]).state).toBe("disabled");
  });

  it("describes stdio as ready on demand instead of stopped", () => {
    const summary = summarizeAgentAccess(status("stdio", true), [workspace], [storage]);
    expect(summary.state).toBe("ready_stdio");
    expect(summary.label).toContain("on demand");
  });

  it("separates HTTP stopped and running states", () => {
    expect(summarizeAgentAccess(status("http", false), [workspace], [storage]).state).toBe("http_stopped");
    expect(summarizeAgentAccess(status("http", true, true), [workspace], [storage]).state).toBe("http_running");
  });

  it("flags missing workspace exposure as needs attention", () => {
    const hiddenStorage = { ...storage, mcpExposed: false };
    expect(summarizeAgentAccess(status("stdio", true), [workspace], [hiddenStorage]).state).toBe("needs_attention");
  });

  it("does not call a broad/manual grant workspace-ready", () => {
    const manualStorage = {
      ...storage,
      mcpPolicy: {
        ...storage.mcpPolicy,
        rules: [
          {
            id: "manual-rule",
            prefix: "/",
            access: "read_only" as const,
            source: { kind: "manual" as const },
          },
        ],
      },
    };
    expect(summarizeAgentAccess(status("stdio", true), [workspace], [manualStorage]).state).toBe("needs_attention");
  });
});
