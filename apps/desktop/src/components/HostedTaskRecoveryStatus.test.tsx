import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HostedTaskRecoveryStatus } from "./HostedTaskRecoveryStatus";
import { listHostedTaskExecutionStatus, reconcileDeletedHostedAttempt } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  listHostedTaskExecutionStatus: vi.fn(),
  reconcileDeletedHostedAttempt: vi.fn(),
}));

const empty = {
  workspaceId: "workspace-1",
  taskId: "task-1",
  attempts: [],
};

const unresolved = {
  ...empty,
  attempts: [{
    executionId: "44444444-4444-4444-8444-444444444444",
    status: "running" as const,
    startedAt: "2026-11-10T00:00:00Z",
    finishedAt: null,
    recovery: "manual_session_reconciliation" as const,
    providerSessionRecorded: false,
  }],
};

describe("HostedTaskRecoveryStatus", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listHostedTaskExecutionStatus).mockResolvedValue(empty);
  });

  it("reads only private status and exposes no upload, publish or execute buttons", async () => {
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    expect(await screen.findByText(/No hosted execution has been recorded/)).toBeInTheDocument();
    expect(listHostedTaskExecutionStatus).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      taskId: "task-1",
    });
    fireEvent.click(screen.getByRole("button", { name: "Refresh status" }));
    await waitFor(() => expect(listHostedTaskExecutionStatus).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("button", { name: /^(execute|upload|publish)/i })).not.toBeInTheDocument();
  });

  it("warns not to retry a possibly billed session whose ID is unknown", async () => {
    vi.mocked(listHostedTaskExecutionStatus).mockResolvedValueOnce(unresolved);
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    expect(await screen.findByText(/Do not retry creation/)).toBeInTheDocument();
    expect(screen.getByTestId("hosted-execution-status")).not.toHaveTextContent("sess_secret");
  });

  it("distinguishes verified public API deletion from physical erasure", async () => {
    vi.mocked(listHostedTaskExecutionStatus).mockResolvedValueOnce({
      ...empty,
      attempts: [{
        ...unresolved.attempts[0],
        recovery: "public_session_deleted_needs_local_review",
        providerSessionRecorded: true,
      }],
    });
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    expect(await screen.findByText(/Physical erasure is not confirmed/)).toBeInTheDocument();
    expect(screen.getByTestId("hosted-execution-status")).not.toHaveTextContent("sess_secret");
  });

  it("offers local reconciliation only after a verified deletion receipt", async () => {
    const cleaned = { ...unresolved.attempts[0], recovery: "public_session_deleted_needs_local_review" as const };
    vi.mocked(listHostedTaskExecutionStatus)
      .mockResolvedValueOnce({ ...empty, attempts: [cleaned] })
      .mockResolvedValueOnce({ ...empty, attempts: [{
        ...cleaned, status: "failed" as const, recovery: "public_session_deletion_confirmed" as const,
      }] });
    vi.mocked(reconcileDeletedHostedAttempt).mockResolvedValue(true);
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    fireEvent.click(await screen.findByRole("button", { name: "Resolve local attempt" }));
    await waitFor(() => expect(reconcileDeletedHostedAttempt).toHaveBeenCalledWith({
      workspaceId: "workspace-1", taskId: "task-1", executionId: cleaned.executionId,
    }));
    expect(await screen.findByText(/Physical erasure and retention expiry are not confirmed/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Resolve local attempt" })).not.toBeInTheDocument();
  });

  it("rejects stale workspace history after the selected task changes", async () => {
    let finishOld: ((value: typeof unresolved) => void) | undefined;
    vi.mocked(listHostedTaskExecutionStatus)
      .mockReturnValueOnce(new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce({ ...empty, workspaceId: "workspace-2", taskId: "task-2" });
    const view = render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    await waitFor(() => expect(listHostedTaskExecutionStatus).toHaveBeenCalledTimes(1));
    view.rerender(<HostedTaskRecoveryStatus workspaceId="workspace-2" taskId="task-2" />);
    expect(await screen.findByText(/No hosted execution has been recorded/)).toBeInTheDocument();
    finishOld?.(unresolved);
    await Promise.resolve();
    expect(screen.queryByText(/Do not retry creation/)).not.toBeInTheDocument();
  });

  it("never renders potentially sensitive native provider errors", async () => {
    vi.mocked(listHostedTaskExecutionStatus).mockRejectedValueOnce(
      new Error("sess_secret_do_not_render"),
    );
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("history is unavailable");
    expect(screen.getByTestId("hosted-execution-status").outerHTML).not.toContain("sess_secret");
  });
});
