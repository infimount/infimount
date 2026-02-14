import { render, screen, waitFor } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { FileBrowser } from "./FileBrowser";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createDirectory, listEntries, TauriApiError } from "@/lib/api";
import { AppZoomProvider } from "@/hooks/use-app-zoom";

// Mock the api module
vi.mock("@/lib/api", () => ({
    listEntries: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
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

vi.mock("./WindowControls", () => ({
    WindowControls: () => null,
}));

// Mock UI components that might cause issues in jsdom
vi.mock("@/components/ui/toast", () => ({
    toast: vi.fn(),
}));

describe("FileBrowser Error Handling", () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it("displays user-friendly message for NOT_FOUND error", async () => {
        (listEntries as any)
            .mockResolvedValueOnce([])
            .mockRejectedValueOnce(new TauriApiError("Raw error", "NOT_FOUND"));

        render(
            <AppZoomProvider>
                <FileBrowser sourceId="test" storageName="Test Storage" />
            </AppZoomProvider>
        );

        // Navigate to a missing path (root NOT_FOUND is treated as empty).
        await waitFor(() => {
            expect(listEntries).toHaveBeenCalled();
        });

        fireEvent.click(screen.getByTitle("Click to edit path"));
        fireEvent.change(screen.getByDisplayValue("/"), { target: { value: "/missing" } });
        fireEvent.submit(screen.getByDisplayValue("/missing").closest("form")!);

        await waitFor(() => {
            expect(screen.getByText("Folder not found")).toBeInTheDocument();
            expect(
                screen.getByText("The requested path does not exist on this storage."),
            ).toBeInTheDocument();
        });
    });

    it("displays user-friendly message for PERMISSION_DENIED error", async () => {
        (listEntries as any).mockRejectedValue(new TauriApiError("Raw error", "PERMISSION_DENIED"));

        render(
            <AppZoomProvider>
                <FileBrowser sourceId="test" storageName="Test Storage" />
            </AppZoomProvider>
        );

        await waitFor(() => {
            expect(screen.getByText("Access denied")).toBeInTheDocument();
            expect(
                screen.getByText("You don't have permission to view this location."),
            ).toBeInTheDocument();
        });
    });

    it("displays raw message for unknown errors", async () => {
        (listEntries as any).mockRejectedValue(new TauriApiError("Something went wrong", "UNKNOWN"));

        render(
            <AppZoomProvider>
                <FileBrowser sourceId="test" storageName="Test Storage" />
            </AppZoomProvider>
        );

        await waitFor(() => {
            expect(screen.getByText("Could not connect to this storage")).toBeInTheDocument();
            expect(screen.getByText("Something went wrong")).toBeInTheDocument();
        });
    });
});

describe("FileBrowser shortcuts and creation", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        (listEntries as any).mockResolvedValue([]);
        (createDirectory as any).mockResolvedValue(undefined);
    });

    it("focuses search on Ctrl/Cmd+F", async () => {
        render(
            <AppZoomProvider>
                <FileBrowser sourceId="test" storageName="Test Storage" />
            </AppZoomProvider>
        );

        const search = await screen.findByPlaceholderText("Search...");
        fireEvent.keyDown(window, { key: "f", ctrlKey: true });
        expect(document.activeElement).toBe(search);
    });

    it("opens create folder dialog on Ctrl/Cmd+Shift+N and creates folder", async () => {
        render(
            <AppZoomProvider>
                <FileBrowser sourceId="test" storageName="Test Storage" />
            </AppZoomProvider>
        );

        await waitFor(() => {
            expect(listEntries).toHaveBeenCalledWith("test", "/");
        });

        fireEvent.keyDown(window, { key: "N", ctrlKey: true, shiftKey: true });

        expect(await screen.findByText("Create new folder")).toBeInTheDocument();
        fireEvent.change(screen.getByPlaceholderText("New Folder"), {
            target: { value: "from-shortcut" },
        });
        fireEvent.click(screen.getByRole("button", { name: "Create" }));

        await waitFor(() => {
            expect(createDirectory).toHaveBeenCalledWith("test", "/from-shortcut");
        });
    });
});
