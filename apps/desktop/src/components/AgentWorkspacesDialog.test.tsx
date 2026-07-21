import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWorkspacesDialog } from "./AgentWorkspacesDialog";
import {
  listWorkspaces,
  createWorkspace as apiCreateWorkspace,
  updateWorkspace as apiUpdateWorkspace,
  createDirectory,
  listEntries,
  readFile,
  writeFile,
} from "@/lib/api";
import type { StorageConfig } from "@/types/storage";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    listWorkspaces: vi.fn().mockResolvedValue([]),
    createWorkspace: vi.fn().mockResolvedValue({}),
    updateWorkspace: vi.fn().mockResolvedValue({}),
    createDirectory: vi.fn(),
    listEntries: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
  };
});

vi.mock("@/hooks/use-toast", () => ({
  toast: vi.fn(),
  useToast: () => ({ toast: vi.fn() }),
}));

const storage: StorageConfig = {
  id: "local",
  type: "local-fs",
  name: "Local Docs",
  backend: "local",
  config: { root: "/tmp/docs" },
  enabled: true,
  mcpExposed: true,
  readOnly: false,
  connected: true,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
  mcpPolicy: {
    version: 2,
    default_access: "read_write",
    rules: [],
    allowed_paths: [],
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
};

function makeWorkspace(id: string, name: string) {
  return {
    id,
    storageId: "local",
    name,
    rootPath: `/agent-workspaces/${name.toLowerCase().replace(/\s+/g, "-")}`,
    templateId: "coding" as const,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    memoryFiles: ["memory/tasks.md"],
    checkpointIds: [],
  };
}

describe("AgentWorkspacesDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    vi.mocked(createDirectory).mockResolvedValue(undefined);
    vi.mocked(writeFile).mockResolvedValue(undefined);
    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("# Tasks\n"));
    vi.mocked(listEntries).mockResolvedValue([]);
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (apiCreateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (apiUpdateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
  });

  it("creates a scoped workspace and applies MCP policy", async () => {
    const onUpdateStoragePolicy = vi.fn().mockResolvedValue(undefined);
    const ws = makeWorkspace("ws-1", "Agent Research");
    (apiCreateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue(ws);
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([ws]);

    render(
      <AgentWorkspacesDialog
        open
        storages={[storage]}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
        onUpdateStoragePolicy={onUpdateStoragePolicy}
      />,
    );

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Agent Research" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create workspace" }));

    await waitFor(() => {
      expect(writeFile).toHaveBeenCalledWith(
        "local",
        "/agent-workspaces/agent-research/README.md",
        expect.anything(),
      );
      expect(onUpdateStoragePolicy).toHaveBeenCalledWith(
        "local",
        expect.objectContaining({
          default_access: "read_write",
          rules: [expect.objectContaining({ prefix: "/agent-workspaces/agent-research/", access: "read_only" })],
        }),
      );
    });

    await waitFor(() => {
      expect(screen.getAllByText("Agent Research").length).toBeGreaterThan(0);
    });
  });

  it("appends workspace memory and restores a checkpoint", async () => {
    const ws = makeWorkspace("workspace-1", "Existing workspace");
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([ws]);

    vi.mocked(listEntries).mockResolvedValue([
      {
        path: "memory/tasks.md",
        name: "tasks.md",
        is_dir: false,
        size: 7,
        modified_at: null,
      },
    ]);
    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("# Tasks\n"));

    render(
      <AgentWorkspacesDialog
        open
        storages={[storage]}
        auditEvents={[
          {
            id: "audit-1",
            timestamp: "2026-01-01T00:01:00Z",
            actor_type: "mcp_client",
            mcp_client_id: null,
            session_id: null,
            storage_id: "local",
            storage_name: "Local Docs",
            backend: "local",
            tool_name: "list_dir",
            operation: "list",
            path: "/agent-workspaces/existing/memory",
            version_id: null,
            decision: "allowed",
            confirmation_id: null,
            duration_ms: 1,
            bytes_read: null,
            bytes_written: null,
            error_code: null,
          },
        ]}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getAllByText("Existing workspace").length).toBeGreaterThan(0);
    });

    fireEvent.change(screen.getByLabelText("Memory note"), {
      target: { value: "Follow up" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Append memory" }));

    await waitFor(() => {
      expect(writeFile).toHaveBeenCalledWith(
        "local",
        "/agent-workspaces/existing-workspace/memory/tasks.md",
        expect.anything(),
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Save checkpoint" }));
    await waitFor(() => {
      expect(window.localStorage.getItem("infimount:agent-workspace-checkpoints:v1")).toContain(
        "workspace-1",
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Restore memory" }));
    await waitFor(() => {
      expect(writeFile).toHaveBeenCalledWith(
        "local",
        "/agent-workspaces/existing-workspace/memory/tasks.md",
        expect.anything(),
      );
    });
  });
});
