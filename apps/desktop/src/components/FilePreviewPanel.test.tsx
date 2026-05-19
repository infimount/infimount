import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { FilePreviewPanel } from "./FilePreviewPanel";
import {
  generateDownloadLink,
  getStorageCapabilities,
  readFile,
  statEntry,
  writeFile,
} from "@/lib/api";
import type { FileItem } from "@/types/storage";

vi.mock("@/lib/api", () => ({
  generateDownloadLink: vi.fn(),
  getStorageCapabilities: vi.fn(),
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
    vi.mocked(getStorageCapabilities).mockResolvedValue({
      list_with_versions: false,
      read_with_version: false,
      delete_with_version: false,
      presign_read: false,
    });
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
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

  it("creates and copies a presigned download link when supported", async () => {
    const file: FileItem = {
      id: "/report.txt",
      name: "report.txt",
      type: "file",
      extension: "txt",
      size: 128,
      modified: new Date(),
    };

    vi.mocked(getStorageCapabilities).mockResolvedValue({
      list_with_versions: false,
      read_with_version: false,
      delete_with_version: false,
      presign_read: true,
    });
    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("report"));
    vi.mocked(generateDownloadLink).mockResolvedValue("https://example.test/report?signature=abc");

    render(
      <FilePreviewPanel
        file={file}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={() => undefined}
      />,
    );

    const button = await screen.findByRole("button", { name: /Create link/i });
    fireEvent.click(button);

    await waitFor(() => {
      expect(generateDownloadLink).toHaveBeenCalledWith("storage-1", "/report.txt", 900);
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
        "https://example.test/report?signature=abc",
      );
    });
  });
});
