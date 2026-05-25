import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TransferQueuePanel } from "./TransferQueuePanel";
import { TransferQueueProvider, useTransferQueue } from "@/hooks/use-transfer-queue";
import { planTransferEntries, transferEntries } from "@/lib/api";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    planTransferEntries: vi.fn().mockResolvedValue({
      operation: "copy",
      conflictPolicy: "fail",
      entries: [],
      summary: {
        create: 2,
        overwrite: 0,
        skip: 0,
        rename: 0,
        noop: 0,
        conflict: 0,
        totalItems: 2,
        totalBytes: 42,
      },
    }),
    transferEntries: vi.fn().mockResolvedValue(undefined),
  };
});

function AddTransferButton() {
  const { enqueueTransfer } = useTransferQueue();
  return (
    <button
      type="button"
      onClick={() =>
        enqueueTransfer({
          fromSourceId: "local",
          toSourceId: "archive",
          sourceName: "Local",
          destinationName: "Archive",
          paths: ["/photo.png", "/notes.txt"],
          targetDir: "/incoming",
          operation: "copy",
          conflictPolicy: "fail",
        })
      }
    >
      Add transfer
    </button>
  );
}

function renderPanel() {
  render(
    <TransferQueueProvider>
      <AddTransferButton />
      <TransferQueuePanel />
    </TransferQueueProvider>,
  );
}

describe("TransferQueuePanel", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(planTransferEntries).mockReset();
    vi.mocked(planTransferEntries).mockResolvedValue({
      operation: "copy",
      conflictPolicy: "fail",
      entries: [],
      summary: {
        create: 2,
        overwrite: 0,
        skip: 0,
        rename: 0,
        noop: 0,
        conflict: 0,
        totalItems: 2,
        totalBytes: 42,
      },
    });
    vi.mocked(transferEntries).mockReset();
  });
  it("renders transfer status, route, and clear action", async () => {
    vi.mocked(transferEntries).mockResolvedValueOnce(undefined);
    renderPanel();

    expect(screen.queryByLabelText("Transfer queue")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    expect(await screen.findByLabelText("Transfer queue")).toBeInTheDocument();
    expect(screen.getByText("Copy 2 items")).toBeInTheDocument();
    expect(screen.getByText(/Local → Archive/)).toBeInTheDocument();

    await waitFor(() => expect(screen.getByText("Done")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Clear done" }));

    expect(screen.queryByLabelText("Transfer queue")).not.toBeInTheDocument();
  });

  it("offers retry for failed transfers", async () => {
    vi.mocked(transferEntries).mockRejectedValueOnce(new Error("temporary backend failure"));
    renderPanel();

    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    expect(await screen.findByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("temporary backend failure")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry transfer" })).toBeInTheDocument();
  });
});
