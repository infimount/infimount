import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentTaskHostedExportPreview } from "./AgentTaskHostedExportPreview";
import { previewHostedTaskExport, verifyHostedTaskExport } from "@/lib/agentTasks";

vi.mock("@/lib/agentTasks", () => ({
  previewHostedTaskExport: vi.fn(),
  verifyHostedTaskExport: vi.fn(),
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

const verified = {
  inputFileCount: 1,
  inputTotalBytes: 5,
  bundleMetadataSha256: "b".repeat(64),
  verifiedBytes: true as const,
  exportAuthorized: false as const,
};

describe("AgentTaskHostedExportPreview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(previewHostedTaskExport).mockResolvedValue(preview);
    vi.mocked(verifyHostedTaskExport).mockResolvedValue(verified);
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

  it("invalidates an in-flight preview even when edited text is restored", async () => {
    let complete: ((value: typeof preview) => void) | undefined;
    const pending = new Promise<typeof preview>((resolve) => {
      complete = resolve;
    });
    vi.mocked(previewHostedTaskExport).mockReturnValueOnce(pending);
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    expect(screen.getByRole("button", { name: /Checking scope/i })).toBeDisabled();

    // A -> B -> A must not make response A from the first request valid again.
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "A different objective" },
    });
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "Summarize only the CSV." },
    });
    expect(screen.getByRole("button", { name: /Preview export scope/i })).not.toBeDisabled();
    await act(async () => {
      complete?.(preview);
      await pending;
    });
    expect(screen.queryByTestId("hosted-export-preview")).not.toBeInTheDocument();
    expect(previewHostedTaskExport).toHaveBeenCalledTimes(1);
  });

  it("rehashes actual prepared input bytes in native code without issuing an upload", async () => {
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    expect(await screen.findByTestId("hosted-export-preview")).toBeInTheDocument();
    expect(verifyHostedTaskExport).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Verify actual prepared bytes/i }));
    await waitFor(() => expect(verifyHostedTaskExport).toHaveBeenCalledWith({
      workspaceId: "workspace",
      taskId: "task-1",
      reviewedExportBrief: {
        title: "Approved sample",
        objective: "Summarize only the CSV.",
        requestedOutputs: ["outputs/report.md"],
      },
    }));
    expect(await screen.findByTestId("hosted-export-verified")).toHaveTextContent(
      /not export or spending approval/i,
    );
    expect(screen.queryByRole("button", { name: /^(execute|upload|publish)/i })).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "Changed instructions" },
    });
    expect(screen.queryByTestId("hosted-export-verified")).not.toBeInTheDocument();
  });

  it("rejects a verified result whose actual scope mismatches the reviewed digest", async () => {
    vi.mocked(verifyHostedTaskExport).mockResolvedValueOnce({
      ...verified,
      bundleMetadataSha256: "c".repeat(64),
    });
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    await screen.findByTestId("hosted-export-preview");
    fireEvent.click(screen.getByRole("button", { name: /Verify actual prepared bytes/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("scope changed");
    expect(screen.queryByTestId("hosted-export-verified")).not.toBeInTheDocument();
  });

  it("never accepts byte-verification evidence after the reviewed brief is changed", async () => {
    let finish: ((value: typeof verified) => void) | undefined;
    const pending = new Promise<typeof verified>((resolve) => {
      finish = resolve;
    });
    vi.mocked(verifyHostedTaskExport).mockReturnValueOnce(pending);
    render(<AgentTaskHostedExportPreview workspaceId="workspace" taskId="task-1" />);
    fill();
    fireEvent.click(screen.getByRole("button", { name: /Preview export scope/i }));
    await screen.findByTestId("hosted-export-preview");
    fireEvent.click(screen.getByRole("button", { name: /Verify actual prepared bytes/i }));
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "Changed, then restored" },
    });
    fireEvent.change(screen.getByLabelText("Reviewed objective"), {
      target: { value: "Summarize only the CSV." },
    });
    await act(async () => {
      finish?.(verified);
      await pending;
    });
    expect(screen.queryByTestId("hosted-export-verified")).not.toBeInTheDocument();
  });
});
