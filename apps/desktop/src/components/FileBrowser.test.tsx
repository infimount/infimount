import { render, screen, waitFor } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { FileBrowser } from "./FileBrowser";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createDirectory, listEntries, readFile, TauriApiError, transferEntries } from "@/lib/api";
import { AppZoomProvider } from "@/hooks/use-app-zoom";
import { FileClipboardProvider } from "@/hooks/use-file-clipboard";

function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((promiseResolve, promiseReject) => {
        resolve = promiseResolve;
        reject = promiseReject;
    });
    return { promise, resolve, reject };
}

function renderFileBrowser(sourceId = "test", storageName = "Test Storage") {
    return render(
        <AppZoomProvider>
            <FileClipboardProvider>
                <FileBrowser sourceId={sourceId} storageName={storageName} />
            </FileClipboardProvider>
        </AppZoomProvider>,
    );
}

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

vi.mock("./FileGrid", () => ({
    FileGrid: ({ files, selectedFiles, canPaste, onSelectFile, onDownloadFile, onCutSelected, onPaste }: {
        files: Array<{ id: string; name: string; type: string }>;
        selectedFiles?: Set<string>;
        canPaste?: boolean;
        onSelectFile?: (id: string) => void;
        onDownloadFile?: (file: { id: string; name: string; type: string }) => void;
        onCutSelected?: () => void;
        onPaste?: () => void;
    }) => (
        <div>
            {canPaste && <span>clipboard ready</span>}
            <button type="button" onClick={() => onCutSelected?.()}>
                Cut selected
            </button>
            <button type="button" disabled={!canPaste} onClick={() => onPaste?.()}>
                Paste selected
            </button>
            {files.map((file) => (
                <div key={file.id}>
                    <button type="button" onClick={() => onSelectFile?.(file.id)}>
                        {file.name}
                    </button>
                    {selectedFiles?.has(file.id) && <span>selected {file.name}</span>}
                    {file.type === "file" && (
                        <button type="button" onClick={() => onDownloadFile?.(file)}>
                            Download {file.name}
                        </button>
                    )}
                </div>
            ))}
        </div>
    ),
}));

vi.mock("./FileTable", () => ({
    FileTable: ({ files, selectedFiles, canPaste, onSelectFile, onDownloadFile, onCutSelected, onPaste }: {
        files: Array<{ id: string; name: string; type: string }>;
        selectedFiles?: Set<string>;
        canPaste?: boolean;
        onSelectFile?: (id: string) => void;
        onDownloadFile?: (file: { id: string; name: string; type: string }) => void;
        onCutSelected?: () => void;
        onPaste?: () => void;
    }) => (
        <div>
            {canPaste && <span>clipboard ready</span>}
            <button type="button" onClick={() => onCutSelected?.()}>
                Cut selected
            </button>
            <button type="button" disabled={!canPaste} onClick={() => onPaste?.()}>
                Paste selected
            </button>
            {files.map((file) => (
                <div key={file.id}>
                    <button type="button" onClick={() => onSelectFile?.(file.id)}>
                        {file.name}
                    </button>
                    {selectedFiles?.has(file.id) && <span>selected {file.name}</span>}
                    {file.type === "file" && (
                        <button type="button" onClick={() => onDownloadFile?.(file)}>
                            Download {file.name}
                        </button>
                    )}
                </div>
            ))}
        </div>
    ),
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
        vi.mocked(listEntries)
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
        vi.mocked(listEntries).mockRejectedValue(new TauriApiError("Raw error", "PERMISSION_DENIED"));

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
        vi.mocked(listEntries).mockRejectedValue(new TauriApiError("Something went wrong", "UNKNOWN"));

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

    it("ignores errors from a storage load after switching to another storage", async () => {
        const slowLoad = deferred<Awaited<ReturnType<typeof listEntries>>>();

        vi.mocked(listEntries).mockImplementation((sourceId) => {
            if (sourceId === "slow") {
                return slowLoad.promise;
            }
            return Promise.resolve([]);
        });

        const { rerender } = render(
            <AppZoomProvider>
                <FileBrowser sourceId="slow" storageName="Slow Storage" />
            </AppZoomProvider>,
        );

        await waitFor(() => {
            expect(listEntries).toHaveBeenCalledWith("slow", "/");
        });

        rerender(
            <AppZoomProvider>
                <FileBrowser sourceId="fast" storageName="Fast Storage" />
            </AppZoomProvider>,
        );

        await waitFor(() => {
            expect(listEntries).toHaveBeenCalledWith("fast", "/");
        });

        slowLoad.reject(new TauriApiError("Slow storage failed", "PERMISSION_DENIED"));

        await waitFor(() => {
            expect(screen.getByText("This folder is empty")).toBeInTheDocument();
        });
        expect(screen.queryByText("Access denied")).not.toBeInTheDocument();
        expect(screen.queryByText("You don't have permission to view this location.")).not.toBeInTheDocument();
    });
});

describe("FileBrowser shortcuts and creation", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        vi.mocked(listEntries).mockResolvedValue([]);
        vi.mocked(createDirectory).mockResolvedValue(undefined);
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

describe("FileBrowser transfer and download flows", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        vi.mocked(listEntries).mockResolvedValue([
            {
                path: "/report.txt",
                name: "report.txt",
                is_dir: false,
                size: 12,
                modified_at: null,
            },
        ]);
        vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("hello"));
        vi.mocked(transferEntries).mockResolvedValue(undefined);
    });

    it("downloads a file through the file context menu", async () => {
        const createObjectURL = vi
            .spyOn(URL, "createObjectURL")
            .mockReturnValue("blob:infimount-report");
        const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
        const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);

        renderFileBrowser();

        fireEvent.click(await screen.findByText("Download report.txt"));

        await waitFor(() => {
            expect(readFile).toHaveBeenCalledWith("test", "/report.txt");
            expect(createObjectURL).toHaveBeenCalled();
            expect(click).toHaveBeenCalled();
            expect(revokeObjectURL).toHaveBeenCalledWith("blob:infimount-report");
        });
    });

    it("moves selected files with cut and paste actions", async () => {
        renderFileBrowser();

        fireEvent.click(await screen.findByText("report.txt"));
        await screen.findByText("selected report.txt");
        fireEvent.click(screen.getByRole("button", { name: "Cut selected" }));
        await screen.findByText("clipboard ready");
        fireEvent.click(screen.getByRole("button", { name: "Paste selected" }));

        await waitFor(() => {
            expect(transferEntries).toHaveBeenCalledWith(
                "test",
                "test",
                ["/report.txt"],
                "/",
                "move",
                "fail",
            );
        });
    });
});
