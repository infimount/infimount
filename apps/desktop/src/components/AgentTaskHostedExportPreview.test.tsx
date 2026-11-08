import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskHostedExportPreview } from "./AgentTaskHostedExportPreview";
import { previewHostedTaskExport } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  previewHostedTaskExport: vi.fn(),
}));

const preview = {
  provider: "openai_agents_api" as const,
  dataResidency: "United States" as const,
  zeroDataRetentionSupported: false as const,
  sandboxNetworkAccess: "disabled" as const,
  pilotSpendLimitUsdCents: 100,
  reviewedTitle: "Approved sample",
  reviewedInstructions: "Summarize only the CSV.",
  requestedOutputs: ["outputs/report.md"],
  inputs: [
    {
      taskPath: "inputs/sample.csv",
      byteSize: 5,
      sha256: "a".repeat(64),
    },
  ],
  inputFileCount: 1,
  inputTotalBytes: 5,
  bundleMetadataSha256: "b".repeat(64),
  inputsRehashed: false as const,
  exportAuthorized: false as const,
  executable: false as const,
};

function fill(open = true) {
  if (open) fireEvent.click(screen.getByRole("button", { name: /Review remote sandbox export/i }));
  fireEvent.change(screen.getByLabelText("Reviewed export title"), {
    target: { value: "Approved sample" },
  });
  fireEvent.change(screen.getByLabelText("Reviewed objective"), {
    target: { value: "Summarize only the CSV." },
  });
  fireEvent.change(screen.getByLabelText("Requested outputs"), {
    target: { value: "report.md" },
  });
}

describe("AgentTaskHostedExportPreview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(previewHostedTaskExport).mockResolvedValue(preview);
  });

  it("never starts a network or task execution and requires reviewed text", async () => {
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    expect(previewHostedTaskExport).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Review remote sandbox export/i }));
    expect(screen.getByRole("button", { name: /Preview export scope/i })).toBeDisabled();
    expect(screen.getByText(/not an upload or consent/i)).toBeInTheDocument();
    fill(false);
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    await waitFor(() => {
      expect(previewHostedTaskExport).toHaveBeenCalledWith({
        workspaceId: "workspace",
        taskId: "task-1",
        reviewedExportBrief: {
          title: "Approved sample",
          objective: "Summarize only the CSV.",
          requestedOutputs: ["outputs/report.md"],
        },
      });
    });
    expect(await screen.findByTestId("hosted-export-preview")).toHaveTextContent("inputs/sample.csv");
    expect(screen.getByText(/No data transferred/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^(execute|upload|publish)/i })).not.toBeInTheDocument();
  });

  it("hides the previous digest as soon as the reviewed instructions change", async () => {
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    expect(await screen.findByTestId("hosted-export-preview")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "New instructions" },
    });
    expect(screen.queryByTestId("hosted-export-preview")).not.toBeInTheDocument();
  });

  it("shows a sanitized failure without pretending an export was approved", async () => {
    vi.mocked(previewHostedTaskExport).mockRejectedValueOnce(new Error("Prepared inputs exceed the inline pilot limits"));
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Prepared inputs exceed");
    expect(screen.queryByTestId("hosted-export-preview")).not.toBeInTheDocument();
  });
});
