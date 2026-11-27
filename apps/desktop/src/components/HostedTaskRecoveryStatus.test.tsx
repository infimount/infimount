import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HostedTaskRecoveryStatus } from "./HostedTaskRecoveryStatus";
import {
  listHostedTaskExecutionStatus,
  reconcileDeletedHostedAttempt,
  cleanupPinnedHostedSession,
  hostedPilotBuildInfo,
} from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  listHostedTaskExecutionStatus: vi.fn(),
  reconcileDeletedHostedAttempt: vi.fn(),
  cleanupPinnedHostedSession: vi.fn(),
  hostedPilotBuildInfo: vi.fn(),
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
    vi.mocked(hostedPilotBuildInfo).mockResolvedValue({
      enabled: false, syntheticOnly: true, provider: "openai_agents_api", maxSpendUsdCents: 100,
    });
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

  it("offers existing-session cleanup only in a compiled opt-in pilot", async () => {
    const pinned = { ...unresolved.attempts[0],
      providerSessionRecorded: true,
      recovery: "inspect_pinned_session" as const,
    };
    vi.mocked(hostedPilotBuildInfo).mockResolvedValueOnce({
      enabled: true, syntheticOnly: true, provider: "openai_agents_api", maxSpendUsdCents: 100,
    });
    vi.mocked(listHostedTaskExecutionStatus)
      .mockResolvedValueOnce({ ...empty, attempts: [pinned] })
      .mockResolvedValueOnce({ ...empty, attempts: [{
        ...pinned, recovery: "public_session_deleted_needs_local_review" as const,
      }] });
    vi.mocked(cleanupPinnedHostedSession).mockResolvedValueOnce(true);
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    const action = await screen.findByRole("button", { name: "Clean up pinned provider session" });
    fireEvent.click(action);
    await waitFor(() => expect(cleanupPinnedHostedSession).toHaveBeenCalledWith({
      workspaceId: "workspace-1", taskId: "task-1", executionId: pinned.executionId,
    }));
    expect(await screen.findByRole("button", { name: "Resolve local attempt" })).toBeInTheDocument();
    expect(screen.getByTestId("hosted-execution-status")).not.toHaveTextContent("sess_secret");
  });

  it("keeps a cleanup in flight across task selection and releases it without a stale refresh", async () => {
    const pinned = { ...unresolved.attempts[0], providerSessionRecorded: true,
      recovery: "inspect_pinned_session" as const };
    const next = { ...pinned, executionId: "55555555-5555-4555-8555-555555555555" };
    vi.mocked(hostedPilotBuildInfo).mockResolvedValueOnce({
      enabled: true, syntheticOnly: true, provider: "openai_agents_api", maxSpendUsdCents: 100,
    });
    vi.mocked(listHostedTaskExecutionStatus)
      .mockResolvedValueOnce({ ...empty, attempts: [pinned] })
      .mockResolvedValueOnce({ workspaceId: "workspace-2", taskId: "task-2", attempts: [next] });
    let finish!: (value: boolean) => void;
    const pending = new Promise<boolean>((resolve) => { finish = resolve; });
    vi.mocked(cleanupPinnedHostedSession).mockReturnValueOnce(pending);
    const view = render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    fireEvent.click(await screen.findByRole("button", { name: "Clean up pinned provider session" }));
    expect(cleanupPinnedHostedSession).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Refresh status" })).toBeDisabled();
    view.rerender(<HostedTaskRecoveryStatus workspaceId="workspace-2" taskId="task-2" />);
    await screen.findByText(`Attempt ${next.executionId}`);
    expect(screen.getByRole("button", { name: "Clean up pinned provider session" })).toBeDisabled();
    expect(screen.getByText(/Changing tasks does not cancel it/i)).toBeInTheDocument();
    await act(async () => { finish(true); await pending; });
    expect(screen.getByRole("button", { name: "Clean up pinned provider session" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Refresh status" })).toBeEnabled();
    expect(screen.queryByText(`Attempt ${pinned.executionId}`)).not.toBeInTheDocument();
    expect(listHostedTaskExecutionStatus).toHaveBeenCalledTimes(2);
    expect(cleanupPinnedHostedSession).toHaveBeenCalledTimes(1);
  });

  it("does not refresh or treat false deletion confirmation as success", async () => {
    const pinned = { ...unresolved.attempts[0], providerSessionRecorded: true,
      recovery: "inspect_pinned_session" as const };
    vi.mocked(hostedPilotBuildInfo).mockResolvedValueOnce({
      enabled: true, syntheticOnly: true, provider: "openai_agents_api", maxSpendUsdCents: 100,
    });
    vi.mocked(listHostedTaskExecutionStatus).mockResolvedValueOnce({ ...empty, attempts: [pinned] });
    vi.mocked(cleanupPinnedHostedSession).mockResolvedValueOnce(false);
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    fireEvent.click(await screen.findByRole("button", { name: "Clean up pinned provider session" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Authenticated cleanup was not confirmed");
    expect(listHostedTaskExecutionStatus).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Refresh status" })).toBeEnabled();
  });

  it("permits local discard only for a never-submitted Prepared attempt", async () => {
    const prepared = { ...unresolved.attempts[0],
      status: "prepared" as const,
      recovery: "requires_fresh_approval" as const,
    };
    vi.mocked(listHostedTaskExecutionStatus)
      .mockResolvedValueOnce({ ...empty, attempts: [prepared] })
      .mockResolvedValueOnce({ ...empty, attempts: [{
        ...prepared, status: "cancelled" as const,
        recovery: "terminal_needs_cleanup_review" as const,
      }] });
    vi.mocked(reconcileDeletedHostedAttempt).mockResolvedValueOnce(true);
    render(<HostedTaskRecoveryStatus workspaceId="workspace-1" taskId="task-1" />);
    const action = await screen.findByRole("button", { name: "Discard unsubmitted attempt" });
    fireEvent.click(action);
    await waitFor(() => expect(reconcileDeletedHostedAttempt).toHaveBeenCalledWith({
      workspaceId: "workspace-1", taskId: "task-1", executionId: prepared.executionId,
    }));
    expect(screen.queryByRole("button", { name: "Discard unsubmitted attempt" })).not.toBeInTheDocument();
    expect(cleanupPinnedHostedSession).not.toHaveBeenCalled();
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
