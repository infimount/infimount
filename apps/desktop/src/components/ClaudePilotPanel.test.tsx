import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ClaudePilotPanel } from "./ClaudePilotPanel";
import { cancelClaudePilot, claudePilotBuildInfo, runClaudePilot } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  claudePilotBuildInfo: vi.fn(),
  runClaudePilot: vi.fn(),
  cancelClaudePilot: vi.fn(),
}));

const props = {
  workspaceId: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127",
  taskId: "81f08176-86e4-40ec-a9a4-a219c4c9b454",
  workspaceMcpExposed: true,
};

describe("ClaudePilotPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(claudePilotBuildInfo).mockResolvedValue({
      enabled: false,
      platformSupported: true,
      requiresExplicitConsent: true,
      note: "Claude pilot disabled",
    });
    vi.mocked(cancelClaudePilot).mockResolvedValue(true);
  });

  afterEach(() => cleanup());

  it("does not expose any pilot control in normal builds", async () => {
    render(<ClaudePilotPanel {...props} />);
    await waitFor(() => expect(claudePilotBuildInfo).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId("claude-pilot-panel")).not.toBeInTheDocument();
    expect(runClaudePilot).not.toHaveBeenCalled();
  });

  it("requires an absolute CLI path, explicit human consent and exposed workspace MCP", async () => {
    vi.mocked(claudePilotBuildInfo).mockResolvedValue({
      enabled: true,
      platformSupported: true,
      requiresExplicitConsent: true,
      note: "Operator pilot",
    });
    vi.mocked(runClaudePilot).mockResolvedValue({
      executionId: "d9700c47-739c-4405-abd8-d272ee2fa81b",
      status: "succeeded",
      fileCount: 1,
      totalBytes: 87,
      errorCode: null,
    });
    render(<ClaudePilotPanel {...props} />);
    expect(await screen.findByTestId("claude-pilot-panel")).toBeInTheDocument();
    const button = screen.getByRole("button", { name: "Run Claude pilot" });
    expect(button).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Absolute Claude Code CLI path"), {
      target: { value: "/opt/claude" },
    });
    expect(button).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", {
      name: /I approve sending prepared task data to Claude/i,
    }));
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => expect(runClaudePilot).toHaveBeenCalledWith({
      workspaceId: props.workspaceId,
      taskId: props.taskId,
      executablePath: "/opt/claude",
      confirmExternalInference: true,
    }));
    expect(await screen.findByTestId("claude-pilot-outcome")).toHaveTextContent(
      "Claude pilot: succeeded",
    );
    expect(screen.getByTestId("claude-pilot-outcome")).toHaveTextContent(
      "successful execution does not approve publication",
    );
  });

  it("cannot run when the workspace MCP boundary is disabled", async () => {
    vi.mocked(claudePilotBuildInfo).mockResolvedValue({
      enabled: true,
      platformSupported: true,
      requiresExplicitConsent: true,
      note: "Operator pilot",
    });
    render(<ClaudePilotPanel {...props} workspaceMcpExposed={false} />);
    expect(await screen.findByTestId("claude-pilot-panel")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Absolute Claude Code CLI path"), {
      target: { value: "/opt/claude" },
    });
    fireEvent.click(screen.getByRole("checkbox", {
      name: /I approve sending prepared task data to Claude/i,
    }));
    expect(screen.getByRole("button", { name: "Run Claude pilot" })).toBeDisabled();
    expect(runClaudePilot).not.toHaveBeenCalled();
  });

  it("supports an explicit stop request while the supervised process is running", async () => {
    vi.mocked(claudePilotBuildInfo).mockResolvedValue({
      enabled: true,
      platformSupported: true,
      requiresExplicitConsent: true,
      note: "Operator pilot",
    });
    let complete: ((value: Awaited<ReturnType<typeof runClaudePilot>>) => void) | undefined;
    vi.mocked(runClaudePilot).mockImplementation(
      () => new Promise((resolve) => { complete = resolve; }),
    );
    const onRunningChange = vi.fn();
    render(<ClaudePilotPanel {...props} onRunningChange={onRunningChange} />);
    await screen.findByTestId("claude-pilot-panel");
    fireEvent.change(screen.getByLabelText("Absolute Claude Code CLI path"), {
      target: { value: "/opt/claude" },
    });
    fireEvent.click(screen.getByRole("checkbox", {
      name: /I approve sending prepared task data to Claude/i,
    }));
    fireEvent.click(screen.getByRole("button", { name: "Run Claude pilot" }));
    expect(onRunningChange).toHaveBeenCalledWith(true);
    fireEvent.click(screen.getByRole("button", { name: "Stop pilot" }));
    await waitFor(() => expect(cancelClaudePilot).toHaveBeenCalledWith({
      workspaceId: props.workspaceId,
      taskId: props.taskId,
    }));
    complete?.({
      executionId: "fd73dd5e-f2b9-4482-899b-2e315a9185fe",
      status: "cancelled",
      fileCount: 0,
      totalBytes: 0,
      errorCode: "ERR_CLAUDE_CANCELLED",
    });
    await waitFor(() => expect(onRunningChange).toHaveBeenCalledWith(false));
    expect(screen.getByTestId("claude-pilot-outcome")).toHaveTextContent("cancelled");
  });
});
