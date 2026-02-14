import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FileBrowser } from "@/components/FileBrowser";
import { AppZoomProvider } from "@/hooks/use-app-zoom";
import { createDirectory, listEntries } from "@/lib/api";

vi.mock("@/lib/api", () => ({
  listEntries: vi.fn(),
  readFile: vi.fn(),
  createDirectory: vi.fn(),
  deletePath: vi.fn(),
  transferEntries: vi.fn(),
  TauriApiError: class extends Error {
    code: string;
    constructor(message: string, code: string) {
      super(message);
      this.code = code;
    }
  },
}));

vi.mock("@/components/WindowControls", () => ({
  WindowControls: () => null,
}));

describe("FileBrowser end-to-end flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (listEntries as any).mockResolvedValue([]);
    (createDirectory as any).mockResolvedValue(undefined);
  });

  it("supports search focus and create-folder flow", async () => {
    render(
      <AppZoomProvider>
        <FileBrowser sourceId="s1" storageName="Storage 1" />
      </AppZoomProvider>,
    );

    await waitFor(() => {
      expect(listEntries).toHaveBeenCalledWith("s1", "/");
    });

    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByPlaceholderText("Search...");
    expect(document.activeElement).toBe(search);
    const emptyState = await screen.findByText("This folder is empty");
    fireEvent.contextMenu(emptyState);
    fireEvent.click(await screen.findByText("New folder"));
    fireEvent.change(await screen.findByPlaceholderText("New Folder"), {
      target: { value: "docs" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(createDirectory).toHaveBeenCalledWith("s1", "/docs");
    });
  });
});
