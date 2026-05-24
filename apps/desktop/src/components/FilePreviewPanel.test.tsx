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

  it("renders image previews through object URLs", async () => {
    const file: FileItem = {
      id: "/image.png",
      name: "image.png",
      type: "file",
      extension: "png",
      size: 16,
      modified: new Date(),
    };
    const createObjectURL = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:image-preview");

    vi.mocked(readFile).mockResolvedValue(new Uint8Array([137, 80, 78, 71]));

    render(
      <FilePreviewPanel
        file={file}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={() => undefined}
      />,
    );

    const image = await screen.findByRole("img", { name: "image.png" });
    expect(image).toHaveAttribute("src", "blob:image-preview");
    expect(createObjectURL).toHaveBeenCalled();

    createObjectURL.mockRestore();
  });

  it("shows unsupported state for known binary files", async () => {
    const file: FileItem = {
      id: "/archive.zip",
      name: "archive.zip",
      type: "file",
      extension: "zip",
      size: 1024,
      modified: new Date(),
    };

    render(
      <FilePreviewPanel
        file={file}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={() => undefined}
      />,
    );

    expect(await screen.findByText("Preview not available for this file type.")).toBeInTheDocument();
    expect(readFile).not.toHaveBeenCalled();
  });

  it("edits text files and confirms overwrite when the remote file changed", async () => {
    const file: FileItem = {
      id: "/notes.txt",
      name: "notes.txt",
      type: "file",
      extension: "txt",
      size: 128,
      modified: new Date("2026-03-13T10:00:00Z"),
    };
    const onEditModeChange = vi.fn();

    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("before"));
    vi.mocked(statEntry)
      .mockResolvedValueOnce({
        path: "/notes.txt",
        name: "notes.txt",
        is_dir: false,
        size: 128,
        modified_at: "2026-03-13T10:00:00Z",
      })
      .mockResolvedValueOnce({
        path: "/notes.txt",
        name: "notes.txt",
        is_dir: false,
        size: 128,
        modified_at: "2026-03-13T10:05:00Z",
      })
      .mockResolvedValueOnce({
        path: "/notes.txt",
        name: "notes.txt",
        is_dir: false,
        size: 129,
        modified_at: "2026-03-13T10:06:00Z",
      })
      .mockResolvedValueOnce({
        path: "/notes.txt",
        name: "notes.txt",
        is_dir: false,
        size: 129,
        modified_at: "2026-03-13T10:06:00Z",
      });
    vi.mocked(writeFile).mockResolvedValue(undefined);

    render(
      <FilePreviewPanel
        file={file}
        sourceId="storage-1"
        onClose={() => undefined}
        onDownload={() => undefined}
        onEditModeChange={onEditModeChange}
      />,
    );

    expect(await screen.findByText("before")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Edit/i }));
    const editor = await screen.findByDisplayValue("before");
    fireEvent.change(editor, { target: { value: "after" } });
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }));

    expect(await screen.findByText("File changed on disk")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Overwrite" }));

    await waitFor(() => {
      expect(writeFile).toHaveBeenCalledWith(
        "storage-1",
        "/notes.txt",
        new TextEncoder().encode("after"),
      );
      expect(onEditModeChange).toHaveBeenCalledWith(false);
    });
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
