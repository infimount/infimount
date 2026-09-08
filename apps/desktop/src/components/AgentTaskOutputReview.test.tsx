import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskOutputReview } from "./AgentTaskOutputReview";
import { reviewAgentTaskOutputs } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  reviewAgentTaskOutputs: vi.fn(),
}));

vi.mock("./AgentTaskPublicationPanel", () => ({
  AgentTaskPublicationPanel: ({ review }: { review: { taskId: string; fileCount: number } }) => (
    <div data-testid="publication-panel">{review.taskId}:{review.fileCount}</div>
  ),
}));

describe("AgentTaskOutputReview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(reviewAgentTaskOutputs).mockResolvedValue({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      taskId: "task-1",
      taskRoot: "tasks/task-1",
      fileCount: 3,
      totalBytes: 70 * 1024,
      files: [
        { taskPath: "outputs/summary.md", byteSize: 24, sha256: "a".repeat(64), preview: "# Summary\nEverything looks good.", previewUnavailableReason: null },
        { taskPath: "outputs/model.bin", byteSize: 1024, sha256: "b".repeat(64), preview: null, previewUnavailableReason: "binary" },
        { taskPath: "outputs/full-report.txt", byteSize: 69 * 1024, sha256: "c".repeat(64), preview: null, previewUnavailableReason: "too_large" },
      ],
    });
  });

  afterEach(() => cleanup());

  it("reviews only the prepared task identity, renders fresh fingerprints, then enables publication", async () => {
    render(<AgentTaskOutputReview workspaceId="workspace-1" taskId="task-1" />);
    expect(screen.queryByTestId("publication-panel")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Review outputs" }));

    await waitFor(() => expect(reviewAgentTaskOutputs).toHaveBeenCalledTimes(1));
    expect(reviewAgentTaskOutputs).toHaveBeenCalledWith({ workspaceId: "workspace-1", taskId: "task-1" });
    const review = await screen.findByTestId("agent-task-output-review");
    expect(review).toHaveTextContent("outputs/summary.md");
    expect(review).toHaveTextContent("# Summary");
    expect(review).toHaveTextContent("SHA-256");
    expect(review).toHaveTextContent("outputs/model.bin");
    expect(review).toHaveTextContent("Binary or control-character content");
    expect(review).toHaveTextContent("outputs/full-report.txt");
    expect(review).toHaveTextContent("disabled above 64 KiB");
    expect(screen.getByTestId("publication-panel")).toHaveTextContent("task-1:3");
  });

  it("shows a refreshable empty state without a publication surface", async () => {
    vi.mocked(reviewAgentTaskOutputs).mockResolvedValueOnce({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      taskId: "task-1",
      taskRoot: "tasks/task-1",
      fileCount: 0,
      totalBytes: 0,
      files: [],
    });
    render(<AgentTaskOutputReview workspaceId="workspace-1" taskId="task-1" />);

    fireEvent.click(screen.getByRole("button", { name: "Review outputs" }));

    expect(await screen.findByText(/No output files yet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh outputs" })).toBeEnabled();
    expect(screen.queryByTestId("publication-panel")).not.toBeInTheDocument();
  });

  it("drops a stale review result when the task identity changes", async () => {
    let resolveFirst: ((value: Awaited<ReturnType<typeof reviewAgentTaskOutputs>>) => void) | undefined;
    vi.mocked(reviewAgentTaskOutputs).mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }));

    const view = render(<AgentTaskOutputReview workspaceId="workspace-1" taskId="task-1" />);
    fireEvent.click(screen.getByRole("button", { name: "Review outputs" }));
    await waitFor(() => expect(reviewAgentTaskOutputs).toHaveBeenCalledTimes(1));

    view.rerender(<AgentTaskOutputReview workspaceId="workspace-1" taskId="task-2" />);
    resolveFirst?.({
      workspaceId: "workspace-1",
      workspaceName: "Agent Scratch",
      taskId: "task-1",
      taskRoot: "tasks/task-1",
      fileCount: 1,
      totalBytes: 1,
      files: [{ taskPath: "outputs/stale.txt", byteSize: 1, sha256: "d".repeat(64), preview: "x", previewUnavailableReason: null }],
    });

    await waitFor(() => expect(screen.getByRole("button", { name: "Review outputs" })).toBeEnabled());
    expect(screen.queryByText("outputs/stale.txt")).not.toBeInTheDocument();
    expect(screen.queryByTestId("publication-panel")).not.toBeInTheDocument();
  });
});
