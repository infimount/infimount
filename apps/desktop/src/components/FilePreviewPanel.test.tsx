import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { FilePreviewPanel } from "./FilePreviewPanel";
import { readFile, statEntry, writeFile } from "@/lib/api";
import type { FileItem } from "@/types/storage";

vi.mock("@/lib/api", () => ({
  readFile: vi.fn(),
  statEntry: vi.fn(),
  writeFile: vi.fn(),
}));

vi.mock("@/hooks/use-toast", () => ({
  toast: vi.fn(),
}));

describe("FilePreviewPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows a large file error without attempting to preview", async () => {
    const largeFile: FileItem = {
      id: "/model.safetensors",
      name: "model.safetensors",
      type: "file",
      extension: "safetensors",
      size: 320 * 1024 * 1024,
      modified: new Date(),
    };

    render(
      <FilePreviewPanel
        file={largeFile}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={() => undefined}
      />,
    );

    expect(await screen.findByText(/too large to preview/i)).toBeInTheDocument();
    expect(readFile).not.toHaveBeenCalled();
  });

  it("renders text previews and forwards the download action", async () => {
    const file: FileItem = {
      id: "/notes.txt",
      name: "notes.txt",
      type: "file",
      extension: "txt",
      size: 128,
      modified: new Date(),
    };
    const onDownload = vi.fn();

    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("hello from preview"));
    vi.mocked(statEntry).mockResolvedValue({
      path: "/notes.txt",
      name: "notes.txt",
      is_dir: false,
      size: 128,
      modified_at: "2026-03-13T10:00:00Z",
    });
    vi.mocked(writeFile).mockResolvedValue(undefined);

    render(
      <FilePreviewPanel
        file={file}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={onDownload}
      />,
    );

    expect(await screen.findByText("hello from preview")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Download/i }));
    expect(onDownload).toHaveBeenCalled();

    expect(screen.getByRole("button", { name: /Edit/i })).toBeInTheDocument();
  });
});
