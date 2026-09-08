import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import { TauriApiError } from "./api";
import {
  launchAgentTaskInCodex,
  prepareAgentTask,
  preflightAgentTask,
  type AgentTaskCodexHandoffOutput,
  type AgentTaskPreparationRequest,
  type AgentTaskPreflightOutput,
  type PrepareAgentTaskOutput,
} from "./agentTasks";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

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

describe("Agent Task desktop API", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

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

  it("preserves a safe backend error code and message", async () => {
    invokeMock.mockRejectedValueOnce({
      code: "CONFIG_ERROR",
      message: "Agent Task source changed after planning",
    });

    const error = await preflightAgentTask(request).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(TauriApiError);
    expect(error).toMatchObject({
      code: "CONFIG_ERROR",
      message: "Agent Task source changed after planning",
    });
  });

  it("redacts secret-bearing or URL-bearing backend error text", async () => {
    invokeMock.mockRejectedValueOnce(new Error("Bearer token leaked at https://example.invalid/?secret=value"));

    const error = await prepareAgentTask(request).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(TauriApiError);
    expect(error).toMatchObject({
      code: "UNKNOWN",
      message: "Agent Task request failed.",
    });
  });
});
