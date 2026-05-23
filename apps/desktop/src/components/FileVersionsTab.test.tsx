import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { FileVersionsTab } from "./FileVersionsTab";
import { deleteFileVersion, listVersions } from "@/lib/api";
import { toast } from "@/hooks/use-toast";

vi.mock("@/lib/api", () => ({
  deleteFileVersion: vi.fn(),
  listVersions: vi.fn(),
}));

vi.mock("@/hooks/use-toast", () => ({
  toast: vi.fn(),
}));

describe("FileVersionsTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("loads versions and downloads the selected version", async () => {
    const onVersionDownload = vi.fn();
    vi.mocked(listVersions).mockResolvedValue({
      path: "/report.txt",
      next_cursor: null,
      versions: [
        {
          version: "v2",
          size_bytes: 2048,
          modified_at: "2026-05-20T10:00:00Z",
          etag: "etag-v2",
        },
        {
          version: "v1",
          size_bytes: null,
          modified_at: null,
          etag: null,
        },
      ],
    });

    render(
      <FileVersionsTab
        sourceId="storage-1"
        path="/report.txt"
        onVersionDownload={onVersionDownload}
      />,
    );

    expect(await screen.findByText("v2")).toBeInTheDocument();
    expect(screen.getByText("v1")).toBeInTheDocument();
    expect(screen.getByText(/2.0 KB/)).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "Download version" })[0]);

    expect(listVersions).toHaveBeenCalledWith("storage-1", "/report.txt");
    expect(onVersionDownload).toHaveBeenCalledWith("v2");
  });

  it("shows empty and error states", async () => {
    vi.mocked(listVersions).mockResolvedValueOnce({
      path: "/empty.txt",
      next_cursor: null,
      versions: [],
    });

    const { rerender } = render(
      <FileVersionsTab
        sourceId="storage-1"
        path="/empty.txt"
        onVersionDownload={() => undefined}
      />,
    );

    expect(await screen.findByText("No previous versions found.")).toBeInTheDocument();

    vi.mocked(listVersions).mockRejectedValueOnce(new Error("Versions are not enabled"));

    rerender(
      <FileVersionsTab
        sourceId="storage-1"
        path="/error.txt"
        onVersionDownload={() => undefined}
      />,
    );

    expect(await screen.findByText("Versions are not enabled")).toBeInTheDocument();
    expect(screen.getByText(/may not support versioning/i)).toBeInTheDocument();
  });

  it("confirms version deletion and removes the deleted version from the list", async () => {
    vi.mocked(listVersions).mockResolvedValue({
      path: "/report.txt",
      next_cursor: null,
      versions: [
        {
          version: "v1",
          size_bytes: 1024,
          modified_at: "2026-05-20T10:00:00Z",
          etag: "etag-v1",
        },
      ],
    });
    vi.mocked(deleteFileVersion).mockResolvedValue({
      path: "/report.txt",
      version: "v1",
      deleted: true,
    });

    render(
      <FileVersionsTab
        sourceId="storage-1"
        path="/report.txt"
        onVersionDownload={() => undefined}
      />,
    );

    expect(await screen.findByText("v1")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Delete version" }));
    expect(await screen.findByText("Delete this file version?")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Delete Version" }));

    await waitFor(() => {
      expect(deleteFileVersion).toHaveBeenCalledWith("storage-1", "/report.txt", "v1");
      expect(screen.queryByText("v1")).not.toBeInTheDocument();
    });
    expect(toast).toHaveBeenCalledWith({
      title: "Version deleted",
      description: "The file version was successfully deleted.",
    });
  });

  it("shows a destructive toast when version deletion fails", async () => {
    vi.mocked(listVersions).mockResolvedValue({
      path: "/report.txt",
      next_cursor: null,
      versions: [
        {
          version: "v1",
          size_bytes: 1024,
          modified_at: "2026-05-20T10:00:00Z",
          etag: "etag-v1",
        },
      ],
    });
    vi.mocked(deleteFileVersion).mockRejectedValue(new Error("Delete denied"));

    render(
      <FileVersionsTab
        sourceId="storage-1"
        path="/report.txt"
        onVersionDownload={() => undefined}
      />,
    );

    expect(await screen.findByText("v1")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete version" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete Version" }));

    await waitFor(() => {
      expect(toast).toHaveBeenCalledWith({
        title: "Failed to delete",
        description: "Delete denied",
        variant: "destructive",
      });
    });
    expect(screen.getByText("v1")).toBeInTheDocument();
  });
});
