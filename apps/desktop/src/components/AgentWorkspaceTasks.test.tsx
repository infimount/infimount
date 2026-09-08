import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWorkspaceTasks } from "./AgentWorkspaceTasks";
import { listAgentTasks } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  listAgentTasks: vi.fn(),
}));

vi.mock("./AgentTaskOutputReview", () => ({
  AgentTaskOutputReview: ({ workspaceId, taskId }: { workspaceId: string; taskId: string }) => (
    <div data-testid="workspace-task-review">{workspaceId}:{taskId}</div>
  ),
}));

const taskOne = {
  taskId: "81f08176-86e4-40ec-a9a4-a219c4c9b454",
  title: "Newest task",
  createdAt: "2026-09-08T12:00:00+00:00",
  taskRoot: "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454",
};

const taskTwo = {
  taskId: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127",
  title: "Older task",
  createdAt: "2026-09-07T12:00:00+00:00",
  taskRoot: "tasks/f8f47aa7-702d-4fd6-8815-84cbdf3b3127",
};

describe("AgentWorkspaceTasks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listAgentTasks).mockResolvedValue({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      tasks: [taskOne, taskTwo],
      truncated: false,
      skippedInvalidTasks: 0,
    });
  });

  it("loads committed tasks from the workspace and opens the newest task for review", async () => {
    render(<AgentWorkspaceTasks workspaceId="workspace-1" />);

    await waitFor(() => expect(listAgentTasks).toHaveBeenCalledWith({ workspaceId: "workspace-1" }));
    expect(await screen.findByRole("button", { name: /Newest task/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Older task/ })).toBeInTheDocument();
    expect(screen.getByTestId("workspace-task-review")).toHaveTextContent(
      `workspace-1:${taskOne.taskId}`,
    );

    fireEvent.click(screen.getByRole("button", { name: /Older task/ }));
    expect(screen.getByTestId("workspace-task-review")).toHaveTextContent(
      `workspace-1:${taskTwo.taskId}`,
    );
  });

  it("reloads tasks when the selected workspace changes and rejects stale results", async () => {
    let resolveFirst: ((value: Awaited<ReturnType<typeof listAgentTasks>>) => void) | undefined;
    vi.mocked(listAgentTasks)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
      )
      .mockResolvedValueOnce({
        workspaceId: "workspace-2",
        workspaceName: "Second",
        tasks: [taskTwo],
        truncated: false,
        skippedInvalidTasks: 0,
      });

    const view = render(<AgentWorkspaceTasks workspaceId="workspace-1" />);
    await waitFor(() => expect(listAgentTasks).toHaveBeenCalledTimes(1));
    view.rerender(<AgentWorkspaceTasks workspaceId="workspace-2" />);
    await waitFor(() => expect(listAgentTasks).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: /Older task/ })).toBeInTheDocument();

    resolveFirst?.({
      workspaceId: "workspace-1",
      workspaceName: "First",
      tasks: [taskOne],
      truncated: false,
      skippedInvalidTasks: 0,
    });
    await Promise.resolve();

    expect(screen.queryByRole("button", { name: /Newest task/ })).not.toBeInTheDocument();
    expect(screen.getByTestId("workspace-task-review")).toHaveTextContent(
      `workspace-2:${taskTwo.taskId}`,
    );
  });

  it("shows bounded-list and malformed-task warnings without exposing publication controls", async () => {
    vi.mocked(listAgentTasks).mockResolvedValueOnce({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      tasks: [taskOne],
      truncated: true,
      skippedInvalidTasks: 2,
    });

    render(<AgentWorkspaceTasks workspaceId="workspace-1" />);

    expect(await screen.findByText(/older or excess entries may not be shown/i)).toBeInTheDocument();
    expect(screen.getByText(/2 malformed task entries were ignored/i)).toBeInTheDocument();
    expect(screen.getByText(/publication remains a separate explicit step/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /publish/i })).not.toBeInTheDocument();
  });

  it("shows an empty state for a workspace with no committed tasks", async () => {
    vi.mocked(listAgentTasks).mockResolvedValueOnce({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      tasks: [],
      truncated: false,
      skippedInvalidTasks: 0,
    });

    render(<AgentWorkspaceTasks workspaceId="workspace-1" />);
    expect(
      await screen.findByText(/No committed Agent Tasks are available in this workspace yet/),
    ).toBeInTheDocument();
  });
});
