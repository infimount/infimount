import { describe, expect, it } from "vitest";

import { isWorkspaceAgentAccessPrepared, summarizeAgentAccess } from "./agentAccessStatus";
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
});

describe("isWorkspaceAgentAccessPrepared", () => {
  it("accepts the exact workspace-managed least-privilege rule", () => {
    expect(isWorkspaceAgentAccessPrepared(workspace, storage)).toBe(true);
  });

  it("rejects a stale managed prefix", () => {
    const stalePrefix: StorageConfig = {
      ...storage,
      mcpPolicy: {
        ...storage.mcpPolicy,
        rules: storage.mcpPolicy.rules.map((rule) => ({
          ...rule,
          prefix: "/agent-workspaces/other",
        })),
      },
    };
    expect(isWorkspaceAgentAccessPrepared(workspace, stalePrefix)).toBe(false);
    expect(summarizeAgentAccess(status("stdio", true), [workspace], [stalePrefix]).state).toBe(
      "needs_attention",
    );
  });

  it("rejects a managed rule with the wrong access mode", () => {
    const wrongAccess: StorageConfig = {
      ...storage,
      mcpPolicy: {
        ...storage.mcpPolicy,
        rules: storage.mcpPolicy.rules.map((rule) => ({
          ...rule,
          access: "read_write" as const,
        })),
      },
    };
    expect(isWorkspaceAgentAccessPrepared(workspace, wrongAccess)).toBe(false);
  });

  it("rejects broad default storage access", () => {
    const broadDefault: StorageConfig = {
      ...storage,
      mcpPolicy: {
        ...storage.mcpPolicy,
        default_access: "read_only",
      },
    };
    expect(isWorkspaceAgentAccessPrepared(workspace, broadDefault)).toBe(false);
  });

  it("rejects an additional positive manual grant", () => {
    const manualGrant: StorageConfig = {
      ...storage,
      mcpPolicy: {
        ...storage.mcpPolicy,
        rules: [
          ...storage.mcpPolicy.rules,
          {
            id: "manual-rule",
            prefix: "/other",
            access: "read_only",
            source: { kind: "manual" },
          },
        ],
      },
    };
    expect(isWorkspaceAgentAccessPrepared(workspace, manualGrant)).toBe(false);
  });

  it("rejects read-write Agent Access on read-only storage", () => {
    const writableWorkspace: WorkspaceRecord = {
      ...workspace,
      accessProfile: "read_write",
    };
    const readOnlyStorage: StorageConfig = {
      ...storage,
      readOnly: true,
      mcpPolicy: {
        ...storage.mcpPolicy,
        rules: storage.mcpPolicy.rules.map((rule) => ({
          ...rule,
          access: "read_write" as const,
        })),
      },
    };
    expect(isWorkspaceAgentAccessPrepared(writableWorkspace, readOnlyStorage)).toBe(false);
  });

  it("rejects unsupported or missing workspace access profiles", () => {
    const unscopedWorkspace: WorkspaceRecord = {
      ...workspace,
      accessProfile: "none",
    };
    expect(isWorkspaceAgentAccessPrepared(unscopedWorkspace, storage)).toBe(false);
  });
});
