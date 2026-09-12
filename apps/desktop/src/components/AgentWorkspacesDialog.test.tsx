import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWorkspacesDialog } from "./AgentWorkspacesDialog";
import {
  listWorkspaces,
  archiveUnsupportedWorkspaces,
  createWorkspaceAtomic as apiCreateWorkspaceAtomic,
  createWorkspaceCheckpointCommand,
  listWorkspaceCheckpoints,
  listEntries,
  readFileRange,
  writeFile,
  deleteWorkspace,
  deleteWorkspaceWithFiles,
} from "@/lib/api";
import {
  prepareWorkspaceStorageBinding,
  workspaceStorageIssue,
} from "@/lib/workspaceStorage";
import type { StorageConfig } from "@/types/storage";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    listWorkspaces: vi.fn().mockResolvedValue([]),
    archiveUnsupportedWorkspaces: vi.fn().mockResolvedValue({ archivedCount: 0, backupPath: null }),
    createWorkspaceAtomic: vi.fn().mockResolvedValue({
      workspace: {},
      policyUpdated: true,
      rollbackErrors: [],
    }),
    createWorkspaceCheckpointCommand: vi.fn().mockResolvedValue({
      schemaVersion: 1,
      id: "checkpoint-1",
      workspaceId: "workspace-1",
      label: "Checkpoint",
      createdAt: "2026-01-01T00:00:00Z",
      manifestPath: "/agent-workspaces/existing-workspace/.infimount/checkpoints/checkpoint-1.json",
      fileCount: 3,
    }),
    listWorkspaceCheckpoints: vi.fn().mockResolvedValue([]),
    restoreWorkspaceCheckpointCommand: vi.fn().mockResolvedValue(undefined),
    listEntries: vi.fn().mockResolvedValue([]),
    readFileRange: vi.fn(),
    writeFile: vi.fn(),
    deleteWorkspace: vi.fn(),
    deleteWorkspaceWithFiles: vi.fn(),
  };
});

vi.mock("@/lib/workspaceStorage", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/workspaceStorage")>();
  return {
    ...actual,
    prepareWorkspaceStorageBinding: vi.fn().mockResolvedValue({
      storageId: "local",
      normalized: false,
    }),
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
  config: { rootPath: "/tmp/docs" },
  enabled: true,
  mcpExposed: true,
  readOnly: false,
  connected: true,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
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
};

function makeWorkspace(
  id: string,
  name: string,
  options: { templateId?: string; accessProfile?: string; memoryFiles?: string[] } = {},
) {
  const templateId = options.templateId ?? "custom";
  return {
    id,
    schemaVersion: 2,
    storageId: "local",
    name,
    rootPath: `/agent-workspaces/${name.toLowerCase().replace(/\s+/g, "-")}`,
    templateId,
    accessProfile: options.accessProfile ?? "read_only",
    policyRuleId: `workspace:${id}`,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    memoryFiles: options.memoryFiles ?? [],
    checkpointIds: [],
  };
}

function renderDialog(storages: StorageConfig[] = [storage]) {
  return render(
    <AgentWorkspacesDialog
      open
      storages={storages}
      onOpenChange={vi.fn()}
      onSelectStorage={vi.fn()}
    />,
  );
}

describe("AgentWorkspacesDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    vi.mocked(prepareWorkspaceStorageBinding).mockResolvedValue({
      storageId: "local",
      normalized: false,
    });
    vi.mocked(readFileRange).mockResolvedValue({
      totalSize: 8,
      offset: 0,
      bytes: Array.from(new TextEncoder().encode("# Tasks\n")),
      truncated: false,
    });
    vi.mocked(listEntries).mockResolvedValue([]);
    vi.mocked(deleteWorkspace).mockResolvedValue(undefined);
    vi.mocked(deleteWorkspaceWithFiles).mockResolvedValue(undefined);
    vi.mocked(listWorkspaceCheckpoints).mockResolvedValue([]);
    vi.mocked(archiveUnsupportedWorkspaces).mockResolvedValue({ archivedCount: 0, backupPath: null });
    vi.mocked(listWorkspaces).mockResolvedValue([]);
  });

  it("creates a plain scoped workspace with an automatic root and mandatory policy", async () => {
    const ws = makeWorkspace("ws-1", "Agent Research");
    vi.mocked(apiCreateWorkspaceAtomic).mockResolvedValue({
      workspace: ws,
      policyUpdated: true,
      rollbackErrors: [],
    });
    vi.mocked(listWorkspaces).mockResolvedValue([ws]);

    renderDialog();

    expect(screen.getByRole("switch", { name: "Allow agent writes" })).not.toBeChecked();
    expect(screen.queryByRole("combobox", { name: "Workspace starter files" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Root path")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Create workspace" })).toBeEnabled();
    });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Agent Research" } });
    expect(screen.getAllByText("/agent-workspaces/agent-research").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Create workspace" }));

    await waitFor(() => {
      expect(prepareWorkspaceStorageBinding).toHaveBeenCalledWith("local");
      expect(apiCreateWorkspaceAtomic).toHaveBeenCalledWith({
        storageId: "local",
        name: "Agent Research",
        rootPath: "/agent-workspaces/agent-research",
        templateId: "custom",
        adoptExisting: undefined,
        accessProfile: "read_only",
        applyPolicy: true,
      });
    });
    expect(vi.mocked(prepareWorkspaceStorageBinding).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(apiCreateWorkspaceAtomic).mock.invocationCallOrder[0],
    );
  });

  it("creates a read-write workspace only after explicit agent-write opt-in", async () => {
    const ws = makeWorkspace("ws-write", "Agent Outputs", { accessProfile: "read_write" });
    vi.mocked(apiCreateWorkspaceAtomic).mockResolvedValue({
      workspace: ws,
      policyUpdated: true,
      rollbackErrors: [],
    });
    vi.mocked(listWorkspaces).mockResolvedValue([ws]);

    renderDialog();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Create workspace" })).toBeEnabled();
    });
    const writeSwitch = screen.getByRole("switch", { name: "Allow agent writes" });
    fireEvent.click(writeSwitch);
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Agent Outputs" } });
    fireEvent.click(screen.getByRole("button", { name: "Create workspace" }));

    await waitFor(() => {
      expect(apiCreateWorkspaceAtomic).toHaveBeenCalledWith(
        expect.objectContaining({ accessProfile: "read_write", applyPolicy: true }),
      );
    });
  });

  it("blocks the exact shell-variable Local Filesystem root that failed the real pilot", async () => {
    const invalid = {
      ...storage,
      config: { rootPath: "$HOME/infimount-agent-task-workspaces" },
    };
    renderDialog([invalid]);

    expect(await screen.findByText(/shell variable syntax/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create workspace" })).toBeDisabled();
    expect(apiCreateWorkspaceAtomic).not.toHaveBeenCalled();
  });

  it("accepts supported tilde aliases and rejects relative, disabled, and read-only storage configurations", () => {
    expect(workspaceStorageIssue({ ...storage, config: { rootPath: "relative/path" } })).toMatch(/not absolute/i);
    expect(workspaceStorageIssue({ ...storage, config: { rootPath: "~/workspace" } })).toBeNull();
    expect(workspaceStorageIssue({ ...storage, enabled: false })).toMatch(/disabled/i);
    expect(workspaceStorageIssue({ ...storage, readOnly: true })).toMatch(/read-only/i);
    expect(workspaceStorageIssue(storage)).toBeNull();
  });

  it("keeps a plain workspace free of the legacy memory/checkpoint surface", async () => {
    const ws = makeWorkspace("plain", "Plain workspace");
    vi.mocked(listWorkspaces).mockResolvedValue([ws]);
    renderDialog();

    expect(await screen.findByText(/plain scoped workspace/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Memory note")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save checkpoint" })).not.toBeInTheDocument();
  });

  it("preserves starter-note and checkpoint creation for existing templated workspaces", async () => {
    const ws = makeWorkspace("workspace-1", "Existing workspace", {
      templateId: "coding",
      memoryFiles: ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"],
    });
    vi.mocked(listWorkspaces).mockResolvedValue([ws]);
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

    renderDialog();

    await screen.findByLabelText("Memory note");
    expect(screen.getByText(/legacy starter: coding/i)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Memory note"), { target: { value: "Follow up" } });
    fireEvent.click(screen.getByRole("button", { name: "Append note" }));

    await waitFor(() => {
      expect(writeFile).toHaveBeenCalledWith(
        "local",
        "/agent-workspaces/existing-workspace/memory/tasks.md",
        expect.anything(),
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Save checkpoint" }));
    await waitFor(() => {
      expect(createWorkspaceCheckpointCommand).toHaveBeenCalledWith("workspace-1", undefined);
    });
  });

  it("archives unsupported workspace metadata and reloads the list", async () => {
    vi.mocked(archiveUnsupportedWorkspaces).mockResolvedValue({
      archivedCount: 2,
      backupPath: "/tmp/workspaces.archived.20260909.json",
    });
    vi.mocked(listWorkspaces).mockResolvedValue([]);

    renderDialog();
    fireEvent.click(screen.getByTitle("Archive unsupported workspaces"));

    await waitFor(() => {
      expect(archiveUnsupportedWorkspaces).toHaveBeenCalledTimes(1);
      expect(vi.mocked(listWorkspaces).mock.calls.length).toBeGreaterThanOrEqual(2);
    });
  });

  it("keeps registration-only deletion distinct from deleting workspace files", async () => {
    const ws = makeWorkspace("workspace-delete", "Delete workspace");
    vi.mocked(listWorkspaces).mockResolvedValue([ws]);
    const { unmount } = renderDialog();

    await screen.findByRole("button", { name: "Remove registration only" });
    fireEvent.click(screen.getByRole("button", { name: "Remove registration only" }));
    const registrationDialog = await screen.findByRole("alertdialog");
    fireEvent.click(within(registrationDialog).getByRole("button", { name: "Remove registration only" }));
    await waitFor(() => expect(deleteWorkspace).toHaveBeenCalledWith("workspace-delete"));
    expect(deleteWorkspaceWithFiles).not.toHaveBeenCalled();
    unmount();

    vi.mocked(listWorkspaces).mockResolvedValue([ws]);
    renderDialog();
    await screen.findByRole("button", { name: "Delete registration and files" });
    fireEvent.click(screen.getByRole("button", { name: "Delete registration and files" }));
    const filesDialog = await screen.findByRole("alertdialog");
    fireEvent.click(within(filesDialog).getByRole("button", { name: "Delete registration and files" }));
    await waitFor(() => expect(deleteWorkspaceWithFiles).toHaveBeenCalledWith("workspace-delete", true));
  });
});
