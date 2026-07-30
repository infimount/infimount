import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWorkspacesDialog } from "./AgentWorkspacesDialog";
import {
  listWorkspaces,
  createWorkspaceAtomic as apiCreateWorkspaceAtomic,
  updateWorkspace as apiUpdateWorkspace,
  createDirectory,
  listEntries,
  readFile,
  writeFile,
  deleteWorkspace,
  deleteWorkspaceWithFiles,
} from "@/lib/api";
import type { StorageConfig } from "@/types/storage";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    listWorkspaces: vi.fn().mockResolvedValue([]),
    createWorkspaceAtomic: vi.fn().mockResolvedValue({
      workspace: {},
      policyUpdated: false,
      rollbackErrors: [],
    }),
    updateWorkspace: vi.fn().mockResolvedValue({}),
    createDirectory: vi.fn(),
    listEntries: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    deleteWorkspace: vi.fn(),
    deleteWorkspaceWithFiles: vi.fn(),
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
    memoryFiles: ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"],
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
    vi.mocked(deleteWorkspace).mockResolvedValue(undefined);
    vi.mocked(deleteWorkspaceWithFiles).mockResolvedValue(undefined);
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (apiCreateWorkspaceAtomic as ReturnType<typeof vi.fn>).mockResolvedValue({
      workspace: {
        id: "ws-1",
        storageId: "local",
        name: "Agent Research",
        rootPath: "/agent-workspaces/agent-research",
        templateId: "coding",
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
        memoryFiles: ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"],
        checkpointIds: [],
      },
      policyUpdated: true,
      rollbackErrors: [],
    });
    (apiUpdateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
  });

  it("creates a scoped workspace and applies MCP policy", async () => {
    const ws = makeWorkspace("ws-1", "Agent Research");
    (apiCreateWorkspaceAtomic as ReturnType<typeof vi.fn>).mockResolvedValue({
      workspace: ws,
      policyUpdated: true,
      rollbackErrors: [],
    });
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([ws]);

    render(
      <AgentWorkspacesDialog
        open
        storages={[storage]}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Agent Research" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create workspace" }));

    await waitFor(() => {
      expect(apiCreateWorkspaceAtomic).toHaveBeenCalledWith(
        expect.objectContaining({ accessProfile: "read_only" }),
      );
    });

    await waitFor(() => {
      expect(screen.getAllByText("Agent Research").length).toBeGreaterThan(0);
    });
  });

  it("offers separate confirmed registration-only and registration-plus-files deletion", async () => {
    const ws = makeWorkspace("workspace-delete", "Delete workspace");
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([ws]);
    const { unmount } = render(
      <AgentWorkspacesDialog
        open
        storages={[storage]}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
      />,
    );
    await screen.findByRole("button", { name: "Remove registration only" });
    fireEvent.click(screen.getByRole("button", { name: "Remove registration only" }));
    const registrationDialog = await screen.findByRole("alertdialog");
    fireEvent.click(
      within(registrationDialog).getByRole("button", { name: "Remove registration only" }),
    );
    await waitFor(() => expect(deleteWorkspace).toHaveBeenCalledWith("workspace-delete"));
    expect(deleteWorkspaceWithFiles).not.toHaveBeenCalled();
    unmount();

    render(
      <AgentWorkspacesDialog
        open
        storages={[storage]}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
      />,
    );
    await screen.findByRole("button", { name: "Delete registration and files" });
    fireEvent.click(screen.getByRole("button", { name: "Delete registration and files" }));
    const filesDialog = await screen.findByRole("alertdialog");
    expect(within(filesDialog).getByText(/permanently deletes/i)).toBeInTheDocument();
    fireEvent.click(
      within(filesDialog).getByRole("button", { name: "Delete registration and files" }),
    );
    await waitFor(() =>
      expect(deleteWorkspaceWithFiles).toHaveBeenCalledWith("workspace-delete", true),
    );
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
        etag: null,
      },
    ]);
    vi.mocked(readFile).mockImplementation(async (_storageId, path) => {
      if (path.includes("/.infimount/checkpoints/")) {
        const manifestWrite = vi.mocked(writeFile).mock.calls.find((call) => call[1] === path);
        if (manifestWrite) return manifestWrite[2] as Uint8Array;
      }
      return new TextEncoder().encode("# Tasks\n");
    });

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

    window.localStorage.setItem(
      "infimount:agent-workspace-checkpoints:v1",
      JSON.stringify([{ workspaceId: "untrusted" }]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save checkpoint" }));
    await waitFor(() => {
      expect(apiUpdateWorkspace).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "workspace-1",
          checkpointIds: [expect.stringMatching(/^checkpoint-/)],
        }),
      );
      expect(window.localStorage.getItem("infimount:agent-workspace-checkpoints:v1")).toBeNull();
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
