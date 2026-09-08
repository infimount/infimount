import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskPrepareDialog } from "./AgentTaskPrepareDialog";
import { listStorages, listWorkspaces } from "@/lib/api";
import {
  launchAgentTaskInCodex,
  prepareAgentTask,
  preflightAgentTask,
  type AgentTaskPreflightOutput,
} from "@/lib/agentTasks";

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  return {
    ...actual,
    listStorages: vi.fn(),
    listWorkspaces: vi.fn(),
  };
});

vi.mock("@/lib/agentTasks", () => ({
  launchAgentTaskInCodex: vi.fn(),
  preflightAgentTask: vi.fn(),
  prepareAgentTask: vi.fn(),
  reviewAgentTaskOutputs: vi.fn(),
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

const preflightEvidence: AgentTaskPreflightOutput = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  selectedItems: 2,
  fileCount: 18,
  directoryCount: 3,
  totalBytes: 12 * 1024 * 1024,
  sourceMcpExposed: true,
  workspaceMcpExposed: true,
  warnings: ["The source storage already has independent MCP exposure."],
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
    vi.mocked(preflightAgentTask).mockResolvedValue(preflightEvidence);
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
    vi.mocked(launchAgentTaskInCodex).mockResolvedValue({
      client: "codex",
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      taskId: "task-1",
      taskRoot: "tasks/task-1",
      launched: true,
    });
  });

  afterEach(() => cleanup());

  it("shows only eligible local read-write workspaces", async () => {
    renderDialog();
    expect(await screen.findByText(/Agent Scratch/)).toBeInTheDocument();
    expect(screen.queryByText(/Remote Workspace/)).not.toBeInTheDocument();
  });

  it("requires preflight before preparation and shows bounded scope evidence", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    expect(screen.queryByRole("button", { name: "Prepare task" })).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("What should the agent do?"), {
      target: { value: "Validate the export and summarize malformed records." },
    });
    fireEvent.change(screen.getByLabelText(/Expected outputs/), {
      target: { value: "summary.md\noutputs/invalid.csv" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review scope" }));

    await waitFor(() => expect(preflightAgentTask).toHaveBeenCalledTimes(1));
    expect(preflightAgentTask).toHaveBeenCalledWith(
      expect.objectContaining({
        sourceStorageId: "remote-1",
        sourcePaths: ["exports/customers.csv", "schema.json"],
        workspaceId: "workspace-1",
        requestedOutputs: ["outputs/summary.md", "outputs/invalid.csv"],
      }),
    );
    expect(await screen.findByTestId("agent-task-preflight")).toHaveTextContent("Scope review");
    expect(screen.getByTestId("agent-task-preflight")).toHaveTextContent("18");
    expect(screen.getByTestId("agent-task-preflight")).toHaveTextContent("12 MiB");
    expect(screen.getByTestId("agent-task-preflight")).toHaveTextContent(
      "Prepare rechecks the source before copying",
    );
    expect(screen.getByText(/independent MCP exposure/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Prepare task" })).toBeEnabled();
  });

  it("invalidates a reviewed request when the objective changes", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    const objective = screen.getByLabelText("What should the agent do?");
    fireEvent.change(objective, { target: { value: "First objective" } });
    fireEvent.click(screen.getByRole("button", { name: "Review scope" }));
    await screen.findByTestId("agent-task-preflight");

    fireEvent.change(objective, { target: { value: "Changed objective" } });
    expect(screen.queryByTestId("agent-task-preflight")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Prepare task" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review scope" })).toBeEnabled();
  });

  it("rejects stale preflight evidence that resolves after the request changes", async () => {
    let resolvePreflight: ((value: AgentTaskPreflightOutput) => void) | undefined;
    vi.mocked(preflightAgentTask).mockReturnValueOnce(
      new Promise<AgentTaskPreflightOutput>((resolve) => {
        resolvePreflight = resolve;
      }),
    );

    renderDialog();
    await screen.findByText(/Agent Scratch/);
    const objective = screen.getByLabelText("What should the agent do?");
    fireEvent.change(objective, { target: { value: "First objective" } });
    fireEvent.click(screen.getByRole("button", { name: "Review scope" }));
    await waitFor(() => expect(preflightAgentTask).toHaveBeenCalledTimes(1));

    fireEvent.change(objective, { target: { value: "Changed while checking" } });
    resolvePreflight?.(preflightEvidence);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Review scope" })).toBeEnabled(),
    );
    expect(screen.queryByTestId("agent-task-preflight")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Prepare task" })).not.toBeInTheDocument();
  });

  it("prepares the exact reviewed request and keeps success visible if the parent clears selection", async () => {
    const view = renderDialog();
    await screen.findByText(/Agent Scratch/);
    fireEvent.change(screen.getByLabelText("What should the agent do?"), {
      target: { value: "Prepare a concise report." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review scope" }));
    await screen.findByTestId("agent-task-preflight");
    fireEvent.click(screen.getByRole("button", { name: "Prepare task" }));

    await waitFor(() => expect(prepareAgentTask).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId("agent-task-prepared")).toHaveTextContent("Task prepared");
    expect(screen.getByTestId("agent-task-prepared")).toHaveTextContent("tasks/task-1");
    expect(screen.getByTestId("agent-task-output-review-section")).toHaveTextContent(
      "Nothing is published until you explicitly select outputs and approve a separate publication plan",
    );

    view.rerender(
      <AgentTaskPrepareDialog
        open
        onOpenChange={vi.fn()}
        sourceId="remote-1"
        storageName="S3"
        selectedPaths={[]}
      />,
    );

    expect(screen.getByTestId("agent-task-prepared")).toHaveTextContent("Task prepared");
    expect(screen.getByText("The task is committed in Agent Scratch.")).toBeInTheDocument();
    expect(screen.queryByText(/0 selected items/)).not.toBeInTheDocument();
  });

  it("hands the committed task to Codex by workspace and task id", async () => {
    renderDialog();
    await screen.findByText(/Agent Scratch/);
    fireEvent.change(screen.getByLabelText("What should the agent do?"), {
      target: { value: "Prepare a concise report." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review scope" }));
    await screen.findByTestId("agent-task-preflight");
    fireEvent.click(screen.getByRole("button", { name: "Prepare task" }));
    await screen.findByTestId("agent-task-prepared");

    fireEvent.click(screen.getByRole("button", { name: "Open in Codex" }));

    await waitFor(() => expect(launchAgentTaskInCodex).toHaveBeenCalledTimes(1));
    expect(launchAgentTaskInCodex).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      taskId: "task-1",
    });
    expect(await screen.findByTestId("agent-task-codex-launched")).toHaveTextContent(
      "Codex handoff opened",
    );
  });

  it("explains when no eligible local workspace exists", async () => {
    vi.mocked(listWorkspaces).mockResolvedValue([ineligibleWorkspace]);
    renderDialog();
    expect(
      await screen.findByText(/Create a read-write Agent Workspace on Local Filesystem/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review scope" })).toBeDisabled();
  });
});
