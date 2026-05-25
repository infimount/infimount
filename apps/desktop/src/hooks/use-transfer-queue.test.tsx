import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const eventMock = vi.hoisted(() => ({
  listener: undefined as undefined | ((event: { payload: unknown }) => void),
  listen: vi.fn((_event: string, callback: (event: { payload: unknown }) => void) => {
    eventMock.listener = callback;
    return Promise.resolve(vi.fn());
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMock.listen,
}));

import { TransferQueueProvider, useTransferQueue } from "./use-transfer-queue";
import { cancelTransferJob, transferEntries } from "@/lib/api";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    cancelTransferJob: vi.fn().mockResolvedValue(undefined),
    transferEntries: vi.fn().mockResolvedValue(undefined),
  };
});

function QueueHarness() {
  const { jobs, enqueueTransfer, retryTransfer, cancelTransfer } = useTransferQueue();
  return (
    <div>
      <button
        type="button"
        onClick={() =>
          enqueueTransfer({
            fromSourceId: "source-a",
            toSourceId: "source-b",
            paths: ["/report.txt"],
            targetDir: "/archive",
            operation: "copy",
            conflictPolicy: "fail",
          })
        }
      >
        Add transfer
      </button>
      <button type="button" onClick={() => jobs[0] && retryTransfer(jobs[0].id)}>
        Retry first
      </button>
      <button
        type="button"
        onClick={() => jobs[jobs.length - 1] && cancelTransfer(jobs[jobs.length - 1].id)}
      >
        Cancel last
      </button>
      <div data-testid="job-count">{jobs.length}</div>
      <div data-testid="first-status">{jobs[0]?.status ?? "none"}</div>
      <div data-testid="last-status">{jobs[jobs.length - 1]?.status ?? "none"}</div>
      <div data-testid="first-progress">{jobs[0]?.progress ?? 0}</div>
    </div>
  );
}

function renderQueue() {
  return render(
    <TransferQueueProvider>
      <QueueHarness />
    </TransferQueueProvider>,
  );
}

describe("useTransferQueue", () => {
  beforeEach(() => {
    eventMock.listener = undefined;
    eventMock.listen.mockClear();
    vi.mocked(cancelTransferJob).mockReset();
    vi.mocked(transferEntries).mockReset();
  });
  it("runs queued transfers and marks them complete", async () => {
    vi.mocked(transferEntries).mockResolvedValueOnce(undefined);
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    await waitFor(() => expect(screen.getByTestId("first-status")).toHaveTextContent("completed"));
    expect(screen.getByTestId("first-progress")).toHaveTextContent("100");
    expect(transferEntries).toHaveBeenCalledWith(
      "source-a",
      "source-b",
      ["/report.txt"],
      "/archive",
      "copy",
      "fail",
      expect.stringMatching(/^transfer-/),
    );
  });

  it("updates running transfer progress from Tauri events", async () => {
    let resolveTransfer!: () => void;
    vi.mocked(transferEntries).mockImplementationOnce(
      () => new Promise<void>((resolve) => {
        resolveTransfer = resolve;
      }),
    );
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));
    await waitFor(() => expect(screen.getByTestId("first-status")).toHaveTextContent("running"));
    const jobId = vi.mocked(transferEntries).mock.calls[0][6];

    eventMock.listener?.({
      payload: {
        jobId,
        completedItems: 0,
        totalItems: 1,
        bytesTransferred: 50,
        totalBytes: 100,
        currentPath: "/report.txt",
      },
    });

    await waitFor(() => expect(screen.getByTestId("first-progress")).toHaveTextContent("50"));
    resolveTransfer();
  });

  it("runs queued transfers sequentially", async () => {
    vi.mocked(transferEntries).mockResolvedValue(undefined);
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));
    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    await waitFor(() => expect(transferEntries).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByTestId("last-status")).toHaveTextContent("completed"));
  });

  it("can retry a failed transfer", async () => {
    vi.mocked(transferEntries)
      .mockRejectedValueOnce(new Error("network reset"))
      .mockResolvedValueOnce(undefined);
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));
    await waitFor(() => expect(screen.getByTestId("first-status")).toHaveTextContent("failed"));

    fireEvent.click(screen.getByRole("button", { name: "Retry first" }));

    await waitFor(() => expect(screen.getByTestId("first-status")).toHaveTextContent("completed"));
    expect(transferEntries).toHaveBeenCalledTimes(2);
  });

  it("requests cancellation for the active transfer", async () => {
    vi.mocked(transferEntries).mockImplementationOnce(
      () => new Promise((resolve) => setTimeout(resolve, 50)),
    );
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));
    await waitFor(() => expect(screen.getByTestId("first-status")).toHaveTextContent("running"));

    fireEvent.click(screen.getByRole("button", { name: "Cancel last" }));

    await waitFor(() => expect(cancelTransferJob).toHaveBeenCalledWith(expect.stringMatching(/^transfer-/)));
  });

  it("cancels queued transfers before they start", async () => {
    vi.mocked(transferEntries).mockImplementationOnce(
      () => new Promise((resolve) => setTimeout(resolve, 50)),
    );
    renderQueue();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));
    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    await waitFor(() => expect(screen.getByTestId("job-count")).toHaveTextContent("2"));
    fireEvent.click(screen.getByRole("button", { name: "Cancel last" }));

    await waitFor(() => expect(screen.getByTestId("last-status")).toHaveTextContent("cancelled"));
  });
});
