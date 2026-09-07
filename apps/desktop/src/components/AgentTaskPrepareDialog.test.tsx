import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskPrepareDialog, normalizeRequestedOutputs } from "./AgentTaskPrepareDialog";
import { listStorages, listWorkspaces } from "@/lib/api";
import { prepareAgentTask, preflightAgentTask } from "@/lib/agentTasks";

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  return {
    ...actual,
    listStorages: vi.fn(),
    listWorkspaces: vi.fn(),
  };
});

vi.mock("@/lib/agentTasks", () => ({
  preflightAgentTask: vi.fn(),
  prepareAgentTask: vi.fn(),
}));

const localStorageRecord = {
  id: "local-1",
  name: "Local Tasks",
  backend: "local",
  type: "local-fs",
  config: {},
  enabled: true,
  mcpExposed: true,
  readOnly: false,
  connected: true,
  createdAt: "2026-09-07T00:00:00Z",
  updatedAt: "2026-09-07T00:00:00Z",
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

const remoteStorageRecord = {
  ...localStorageRecord,
  id: "remote-1",
  name: "S3",
  backend: "s3",
  type: "aws-s3",
};

const eligibleWorkspace = {
  id: "workspace-1",
  schemaVersion: 2,
  storageId: "local-1",
  name: "Agent Scratch",
  rootPath: "agent-scratch",
  templateId: "data-analysis",
  accessProfile: "read_write",
  createdAt: "2026-09-07T00:00:00Z",
  updatedAt: "2026-09-07T00:00:00Z",
  memoryFiles: ["memory/datasets.md", "memory/observations.md", "memory/runbook.md"],
  checkpointIds: [],
};

const ineligibleWorkspace = {
  ...eligibleWorkspace,
  id: "workspace-remote",
  storageId: "remote-1",
  name: "Remote Workspace",
};

function renderDialog() {
  return render(
    <AgentTaskPrepareDialog
      open
      onOpenChange={vi.fn()}
      sourceId="remote-1"
      storageName="S3"
      selectedPaths={["exports/customers.csv", "schema.json"]}
    />,
  );
}

describe("AgentTaskPrepareDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listStorages).mockResolvedValue([
      localStorageRecord as never,
      remoteStorageRecord as never,
    ]);
    vi.mocked(listWorkspaces).mockResolvedValue([eligibleWorkspace, ineligibleWorkspace]);
    vi.mocked(preflightAgentTask).mockResolvedValue({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      selectedItems: 2,
      fileCount: 18,
      directoryCount: 3,
      totalBytes: 12 * 1024 * 1024,
      sourceMcpExposed: true,
      workspaceMcpExposed: true,
      warnings: ["The source storage already has independent MCP exposure."],
    });
    vi.mocked(prepareAgentTask).mockResolvedValue({
      taskId: "task-1",
      taskRoot: "tasks/task-1",
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      preparedFiles: 18,
      totalBytes: 12 * 1024 * 1024,
      sourceMcpExposed: true,
      workspaceMcpExposed: true,
    });
  });

  afterEach(() => cleanup());

  it("normalizes requested output names into outputs/", () => {
    expect(normalizeRequestedOutputs("summary.md\noutputs/errors.csv\n\n")).toEqual([
      "outputs/summary.md",
      "outputs/errors.csv",
    ]);
  });

  it("shows only eligible local read-write workspaces", async () => {
    renderDialog();
    expect(await screen.findByText(/Agent Scratch/)).toBeInTheDocument();
    expect(screen.queryByText(/Remote Workspace/)).not.toBeInTheDocument();
  });

  it("requires preflight before preparation and shows bounded review evidence", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    expect(screen.queryByRole("button", { name: "Prepare task" })).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("What should the agent do?"), {
      target: { value: "Validate the export and summarize malformed records." },
    });
    fireEvent.change(screen.getByLabelText(/Expected outputs/), {
      target: { value: "summary.md\ninvalid.csv" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));

    await waitFor(() => expect(preflightAgentTask).toHaveBeenCalledTimes(1));
    expect(preflightAgentTask).toHaveBeenCalledWith(
      expect.objectContaining({
        sourceStorageId: "remote-1",
        sourcePaths: ["exports/customers.csv", "schema.json"],
        workspaceId: "workspace-1",
        requestedOutputs: ["outputs/summary.md", "outputs/invalid.csv"],
      }),
    );
    expect(await screen.findByTestId("agent-task-preflight")).toHaveTextContent("18");
    expect(screen.getByTestId("agent-task-preflight")).toHaveTextContent("12 MiB");
    expect(screen.getByText(/independent MCP exposure/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Prepare task" })).toBeEnabled();
  });

  it("invalidates a reviewed request when the objective changes", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    const objective = screen.getByLabelText("What should the agent do?");
    fireEvent.change(objective, { target: { value: "First objective" } });
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    await screen.findByTestId("agent-task-preflight");

    fireEvent.change(objective, { target: { value: "Changed objective" } });
    expect(screen.queryByTestId("agent-task-preflight")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Prepare task" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review access" })).toBeEnabled();
  });

  it("prepares the exact reviewed request and renders success", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    fireEvent.change(screen.getByLabelText("What should the agent do?"), {
      target: { value: "Prepare a concise report." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review access" }));
    await screen.findByTestId("agent-task-preflight");
    fireEvent.click(screen.getByRole("button", { name: "Prepare task" }));

    await waitFor(() => expect(prepareAgentTask).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId("agent-task-prepared")).toHaveTextContent("Task prepared");
    expect(screen.getByTestId("agent-task-prepared")).toHaveTextContent("tasks/task-1");
  });

  it("explains when no eligible local workspace exists", async () => {
    vi.mocked(listWorkspaces).mockResolvedValue([ineligibleWorkspace]);
    renderDialog();
    expect(await screen.findByText(/Create a read-write Agent Workspace on Local Filesystem/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review access" })).toBeDisabled();
  });
});
