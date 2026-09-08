import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import { TauriApiError } from "./api";
import {
  launchAgentTaskInCodex,
  listAgentTasks,
  prepareAgentTask,
  preflightAgentTask,
  previewAgentTaskPublication,
  publishAgentTaskOutputs,
  reviewAgentTaskOutputs,
  type AgentTaskCodexHandoffOutput,
  type AgentTaskListOutput,
  type AgentTaskOutputReviewOutput,
  type AgentTaskPreparationRequest,
  type AgentTaskPreflightOutput,
  type AgentTaskPublicationPreview,
  type AgentTaskPublicationRequest,
  type AgentTaskPublicationOutput,
  type PrepareAgentTaskOutput,
} from "./agentTasks";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

const request: AgentTaskPreparationRequest = {
  sourceStorageId: "source-1",
  sourcePaths: ["exports/customers.csv", "schema.json"],
  workspaceId: "workspace-1",
  title: "Validate export",
  objective: "Validate the export and summarize malformed records.",
  requestedOutputs: ["outputs/summary.md"],
};

const preflightResult: AgentTaskPreflightOutput = {
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

const prepareResult: PrepareAgentTaskOutput = {
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  preparedFiles: 18,
  totalBytes: 12 * 1024 * 1024,
  sourceMcpExposed: true,
  workspaceMcpExposed: true,
};

const handoffResult: AgentTaskCodexHandoffOutput = {
  client: "codex",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  launched: true,
};

const listResult: AgentTaskListOutput = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  tasks: [{ taskId: "task-1", title: "Validate export", createdAt: "2026-09-08T00:00:00+00:00", taskRoot: "tasks/task-1" }],
  truncated: false,
  skippedInvalidTasks: 0,
};

const reviewResult: AgentTaskOutputReviewOutput = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  fileCount: 1,
  totalBytes: 12,
  files: [{ taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64), preview: "# Summary\n", previewUnavailableReason: null }],
};

const publicationRequest: AgentTaskPublicationRequest = {
  workspaceId: "workspace-1",
  taskId: "task-1",
  outputs: [{ taskPath: "outputs/summary.md", byteSize: 12, sha256: "a".repeat(64) }],
  destinationStorageId: "destination-1",
  destinationDir: "published",
  conflictPolicy: "fail",
};

const publicationPreview: AgentTaskPublicationPreview = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  destinationStorageId: "destination-1",
  destinationStorageName: "Published",
  destinationDir: "published",
  conflictPolicy: "fail",
  fileCount: 1,
  totalBytes: 12,
  createCount: 1,
  renameCount: 0,
  conflictCount: 0,
  canPublish: true,
  previewToken: "b".repeat(64),
  files: [{ ...publicationRequest.outputs[0], destinationPath: "published/summary.md", action: "create" }],
};

const publicationResult: AgentTaskPublicationOutput = {
  publicationId: "publication-1",
  publishedAt: "2026-09-08T01:00:00+00:00",
  workspaceId: "workspace-1",
  taskId: "task-1",
  destinationStorageId: "destination-1",
  destinationStorageName: "Published",
  receiptPath: "tasks/task-1/publish-receipt-publication-1.json",
  files: publicationPreview.files,
};

describe("Agent Task desktop API", () => {
  beforeEach(() => vi.clearAllMocks());

  it("forwards the exact request to preflight_agent_task", async () => {
    invokeMock.mockResolvedValueOnce(preflightResult);
    await expect(preflightAgentTask(request)).resolves.toEqual(preflightResult);
    expect(invokeMock).toHaveBeenCalledWith("preflight_agent_task", { request });
  });

  it("forwards the exact request to prepare_agent_task", async () => {
    invokeMock.mockResolvedValueOnce(prepareResult);
    await expect(prepareAgentTask(request)).resolves.toEqual(prepareResult);
    expect(invokeMock).toHaveBeenCalledWith("prepare_agent_task", { request });
  });

  it("forwards only task and workspace identity to the Codex handoff command", async () => {
    invokeMock.mockResolvedValueOnce(handoffResult);
    const handoff = { workspaceId: "workspace-1", taskId: "task-1" };
    await expect(launchAgentTaskInCodex(handoff)).resolves.toEqual(handoffResult);
    expect(invokeMock).toHaveBeenCalledWith("launch_agent_task_in_codex", { request: handoff });
  });

  it("lists committed tasks by workspace identity only", async () => {
    invokeMock.mockResolvedValueOnce(listResult);
    const list = { workspaceId: "workspace-1" };
    await expect(listAgentTasks(list)).resolves.toEqual(listResult);
    expect(invokeMock).toHaveBeenCalledWith("list_agent_tasks", { request: list });
  });

  it("forwards only task and workspace identity to output review", async () => {
    invokeMock.mockResolvedValueOnce(reviewResult);
    const review = { workspaceId: "workspace-1", taskId: "task-1" };
    await expect(reviewAgentTaskOutputs(review)).resolves.toEqual(reviewResult);
    expect(invokeMock).toHaveBeenCalledWith("review_agent_task_outputs", { request: review });
  });

  it("forwards reviewed hashes and explicit destination to publication preview", async () => {
    invokeMock.mockResolvedValueOnce(publicationPreview);
    await expect(previewAgentTaskPublication(publicationRequest)).resolves.toEqual(publicationPreview);
    expect(invokeMock).toHaveBeenCalledWith("preview_agent_task_publication", { request: publicationRequest });
  });

  it("applies only the exact publication request and preview token", async () => {
    invokeMock.mockResolvedValueOnce(publicationResult);
    const apply = { publication: publicationRequest, previewToken: publicationPreview.previewToken };
    await expect(publishAgentTaskOutputs(apply)).resolves.toEqual(publicationResult);
    expect(invokeMock).toHaveBeenCalledWith("publish_agent_task_outputs", { request: apply });
  });

  it("preserves a safe backend error code and message", async () => {
    invokeMock.mockRejectedValueOnce({ code: "CONFIG_ERROR", message: "Agent Task source changed after planning" });
    const error = await preflightAgentTask(request).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(TauriApiError);
    expect(error).toMatchObject({ code: "CONFIG_ERROR", message: "Agent Task source changed after planning" });
  });

  it("redacts secret-bearing or URL-bearing backend error text", async () => {
    invokeMock.mockRejectedValueOnce(new Error("Bearer token leaked at https://example.invalid/?secret=value"));
    const error = await prepareAgentTask(request).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(TauriApiError);
    expect(error).toMatchObject({ code: "UNKNOWN", message: "Agent Task request failed." });
  });
});
