import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskPublicationPanel } from "./AgentTaskPublicationPanel";
import {
  previewAgentTaskPublication,
  publishAgentTaskOutputs,
  type AgentTaskOutputReviewOutput,
  type AgentTaskPublicationPreview,
} from "@/lib/agentTasks";
import { listStorages } from "@/lib/api";

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  return { ...actual, listStorages: vi.fn() };
});

vi.mock("@/lib/agentTasks", () => ({
  previewAgentTaskPublication: vi.fn(),
  publishAgentTaskOutputs: vi.fn(),
}));

const review: AgentTaskOutputReviewOutput = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  fileCount: 2,
  totalBytes: 30,
  files: [
    { taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64), preview: "summary", previewUnavailableReason: null },
    { taskPath: "outputs/data.csv", byteSize: 18, sha256: "b".repeat(64), preview: "a,b", previewUnavailableReason: null },
  ],
};

const storage = {
  id: "destination-1",
  name: "Published",
  backend: "local",
  type: "local-fs",
  config: {},
  enabled: true,
  readOnly: false,
  connected: true,
  createdAt: "2026-09-08T00:00:00Z",
  updatedAt: "2026-09-08T00:00:00Z",
};

const preview: AgentTaskPublicationPreview = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  destinationStorageId: "destination-1",
  destinationStorageName: "Published",
  destinationDir: "exports",
  conflictPolicy: "fail",
  fileCount: 1,
  totalBytes: 12,
  createCount: 1,
  renameCount: 0,
  conflictCount: 0,
  canPublish: true,
  previewToken: "c".repeat(64),
  files: [{ taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64), destinationPath: "exports/summary.md", action: "create" }],
};

describe("AgentTaskPublicationPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listStorages).mockResolvedValue([storage as never]);
    vi.mocked(previewAgentTaskPublication).mockResolvedValue(preview);
    vi.mocked(publishAgentTaskOutputs).mockResolvedValue({
      publicationId: "publication-1",
      publishedAt: "2026-09-08T01:00:00Z",
      workspaceId: "workspace-1",
      taskId: "task-1",
      destinationStorageId: "destination-1",
      destinationStorageName: "Published",
      receiptPath: "tasks/task-1/publish-receipt-publication-1.json",
      files: preview.files,
    });
  });

  it("starts with no outputs selected and requires a separate publication preview", async () => {
    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");

    expect(screen.getByText("0 of 2 selected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review publication" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /^Publish / })).not.toBeInTheDocument();
  });

  it("previews the exact reviewed hash and explicit destination before publishing", async () => {
    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "exports" } });
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));

    await waitFor(() => expect(previewAgentTaskPublication).toHaveBeenCalledTimes(1));
    expect(previewAgentTaskPublication).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      taskId: "task-1",
      outputs: [{ taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64) }],
      destinationStorageId: "destination-1",
      destinationDir: "exports",
      conflictPolicy: "fail",
    });
    expect(await screen.findByTestId("agent-task-publication-preview")).toHaveTextContent("exports/summary.md");
    expect(screen.getByRole("button", { name: "Publish 1 approved output" })).toBeEnabled();
  });

  it("applies only the exact reviewed request and preview token", async () => {
    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "exports" } });
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));
    await screen.findByTestId("agent-task-publication-preview");
    fireEvent.click(screen.getByRole("button", { name: "Publish 1 approved output" }));

    await waitFor(() => expect(publishAgentTaskOutputs).toHaveBeenCalledTimes(1));
    expect(publishAgentTaskOutputs).toHaveBeenCalledWith({
      publication: {
        workspaceId: "workspace-1",
        taskId: "task-1",
        outputs: [{ taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64) }],
        destinationStorageId: "destination-1",
        destinationDir: "exports",
        conflictPolicy: "fail",
      },
      previewToken: "c".repeat(64),
    });
    expect(await screen.findByTestId("agent-task-publication-success")).toHaveTextContent("publish-receipt-publication-1.json");
    expect(screen.queryByRole("button", { name: "Publish 1 approved output" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review publication" })).toBeEnabled();
  });

  it("invalidates an approved plan when destination input changes", async () => {
    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "exports" } });
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));
    await screen.findByTestId("agent-task-publication-preview");

    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "changed" } });
    expect(screen.queryByTestId("agent-task-publication-preview")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Publish / })).not.toBeInTheDocument();
  });

  it("discards a stale preview and clears its busy state after request edits", async () => {
    let resolvePreview: ((value: AgentTaskPublicationPreview) => void) | undefined;
    vi.mocked(previewAgentTaskPublication).mockReturnValueOnce(
      new Promise((resolve) => {
        resolvePreview = resolve;
      }),
    );

    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "exports" } });
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));
    await waitFor(() => expect(previewAgentTaskPublication).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Reviewing publication…" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "changed" } });
    expect(screen.getByRole("button", { name: "Review publication" })).toBeEnabled();

    resolvePreview?.(preview);
    await Promise.resolve();
    expect(screen.queryByTestId("agent-task-publication-preview")).not.toBeInTheDocument();
  });

  it("discards publication completion after the reviewed output set is replaced", async () => {
    let resolvePublish: ((value: Awaited<ReturnType<typeof publishAgentTaskOutputs>>) => void) | undefined;
    vi.mocked(publishAgentTaskOutputs).mockReturnValueOnce(
      new Promise((resolve) => {
        resolvePublish = resolve;
      }),
    );

    const view = render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.change(screen.getByLabelText("Destination folder"), { target: { value: "exports" } });
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));
    await screen.findByTestId("agent-task-publication-preview");
    fireEvent.click(screen.getByRole("button", { name: "Publish 1 approved output" }));
    await waitFor(() => expect(publishAgentTaskOutputs).toHaveBeenCalledTimes(1));

    view.rerender(
      <AgentTaskPublicationPanel
        review={{
          ...review,
          totalBytes: 31,
          files: [{ ...review.files[0], byteSize: 13, sha256: "d".repeat(64) }, review.files[1]],
        }}
      />,
    );
    await waitFor(() => expect(screen.getByText("0 of 2 selected")).toBeInTheDocument());

    resolvePublish?.({
      publicationId: "stale-publication",
      publishedAt: "2026-09-08T01:00:00Z",
      workspaceId: "workspace-1",
      taskId: "task-1",
      destinationStorageId: "destination-1",
      destinationStorageName: "Published",
      receiptPath: "tasks/task-1/publish-receipt-stale-publication.json",
      files: preview.files,
    });
    await Promise.resolve();
    expect(screen.queryByTestId("agent-task-publication-success")).not.toBeInTheDocument();
  });

  it("never offers overwrite and blocks apply when fail-mode preview has a conflict", async () => {
    vi.mocked(previewAgentTaskPublication).mockResolvedValueOnce({
      ...preview,
      createCount: 0,
      conflictCount: 1,
      canPublish: false,
      files: [{ ...preview.files[0], action: "conflict" }],
    });
    render(<AgentTaskPublicationPanel review={review} />);
    await screen.findByText("Published");
    fireEvent.click(screen.getByRole("checkbox", { name: "Publish outputs/summary.md" }));
    fireEvent.click(screen.getByRole("button", { name: "Review publication" }));

    expect(await screen.findByText(/Resolve destination conflicts/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Publish / })).not.toBeInTheDocument();
    expect(screen.queryByText(/^Overwrite$/)).not.toBeInTheDocument();
  });
});
