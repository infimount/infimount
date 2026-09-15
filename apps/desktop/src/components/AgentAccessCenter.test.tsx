import type { ComponentProps } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentAccessCenter } from "./AgentAccessCenter";
import type { WorkspaceRecord } from "@/lib/api";
import type { McpClientSnippets, McpRuntimeStatus, StorageConfig } from "@/types/storage";

const workspace: WorkspaceRecord = {
  id: "workspace-1",
  storageId: "storage-1",
  name: "Workspace",
  rootPath: "/agent-workspaces/workspace",
  templateId: "custom",
  accessProfile: "read_only",
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
    rules: [],
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

const snippets: McpClientSnippets = {
  stdio: '{"mcpServers":{"infimount":{"command":"infimount_mcp"}}}',
  http: '{"mcpServers":{"infimount":{"url":"http://127.0.0.1:7331/mcp"}}}',
};

function status(transport: "stdio" | "http", runningHttp = false): McpRuntimeStatus {
  return {
    settings: {
      enabled: true,
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

function renderCenter(
  runtime: McpRuntimeStatus,
  overrides: Partial<ComponentProps<typeof AgentAccessCenter>> = {},
) {
  const props: ComponentProps<typeof AgentAccessCenter> = {
    open: true,
    onOpenChange: vi.fn(),
    status: runtime,
    snippets,
    workspaces: [workspace],
    storages: [storage],
    onPrepare: vi.fn(async () => undefined),
    onVerify: vi.fn(async () => undefined),
    onStartHttp: vi.fn(async () => undefined),
    onStopHttp: vi.fn(async () => undefined),
    onOpenAdvanced: vi.fn(),
    onOpenWorkspaces: vi.fn(),
    ...overrides,
  };
  render(<AgentAccessCenter {...props} />);
  return props;
}

describe("AgentAccessCenter", () => {
  it("describes stdio as client-launched and never offers a Start server button", () => {
    renderCenter(status("stdio"));

    expect(screen.getByText(/stdio is on demand/i)).toBeInTheDocument();
    expect(screen.getByText(/No background server is required/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Start HTTP server/i })).not.toBeInTheDocument();
    expect(screen.getByDisplayValue(snippets.stdio)).toBeInTheDocument();
  });

  it("offers explicit HTTP start controls", () => {
    const stopped = renderCenter(status("http"));
    fireEvent.click(screen.getByRole("button", { name: /Start HTTP server/i }));
    expect(stopped.onStartHttp).toHaveBeenCalledTimes(1);
  });

  it("runs verification and reports a pass", async () => {
    const props = renderCenter(status("stdio"));
    fireEvent.click(screen.getByRole("button", { name: /Verify connection/i }));
    await waitFor(() => expect(props.onVerify).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Verification passed.")).toBeInTheDocument();
  });

  it("routes users without a workspace to workspace creation", () => {
    const props = renderCenter(status("stdio"), { workspaces: [] });
    fireEvent.click(screen.getByRole("button", { name: /Create workspace/i }));
    expect(props.onOpenWorkspaces).toHaveBeenCalledTimes(1);
  });
});
