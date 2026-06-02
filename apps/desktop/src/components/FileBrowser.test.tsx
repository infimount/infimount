import type React from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { FileBrowser } from "./FileBrowser";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
    createDirectory,
    deletePath,
    listEntries,
    listEntriesRecursive,
    readFile,
    TauriApiError,
    planTransferEntries,
    transferEntries,
    writeFile,
} from "@/lib/api";
import { AppZoomProvider } from "@/hooks/use-app-zoom";
import { FileClipboardProvider } from "@/hooks/use-file-clipboard";
import { TransferQueueProvider } from "@/hooks/use-transfer-queue";

function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((promiseResolve, promiseReject) => {
        resolve = promiseResolve;
        reject = promiseReject;
    });
    return { promise, resolve, reject };
}

function withFileBrowserProviders(children: React.ReactNode) {
    return (
        <AppZoomProvider>
            <FileClipboardProvider>
                <TransferQueueProvider>{children}</TransferQueueProvider>
            </FileClipboardProvider>
        </AppZoomProvider>
    );
}

function renderFileBrowser(sourceId = "test", storageName = "Test Storage") {
    return render(
        withFileBrowserProviders(<FileBrowser sourceId={sourceId} storageName={storageName} />),
    );
}

// Mock the api module
vi.mock("@/lib/api", () => ({
    listEntries: vi.fn(),
    listEntriesRecursive: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    createDirectory: vi.fn(),
    deletePath: vi.fn(),
    planTransferEntries: vi.fn().mockResolvedValue({
        operation: "copy",
        conflictPolicy: "fail",
        entries: [],
        summary: {
            create: 1,
            overwrite: 0,
            skip: 0,
            rename: 0,
            noop: 0,
            conflict: 0,
            totalItems: 1,
            totalBytes: 12,
        },
    }),
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

vi.mock("./UploadZone", async () => {
    const React = await import("react");
    return {
        UploadZone: React.forwardRef<{ handleFiles: (files: Array<{ name: string; arrayBuffer: () => Promise<ArrayBuffer> }>) => void }, { onUpload: (files: Array<{ name: string; arrayBuffer: () => Promise<ArrayBuffer> }>) => void; isDragging?: boolean }>(
            ({ onUpload, isDragging }, ref) => {
                React.useImperativeHandle(ref, () => ({ handleFiles: onUpload }), [onUpload]);
                return (
                    <div data-testid="upload-zone" data-dragging={String(Boolean(isDragging))}>
                        <button
                            type="button"
                            onClick={() =>
                                onUpload([
                                    {
                                        name: "fixture.txt",
                                        arrayBuffer: async () => new TextEncoder().encode("fixture").buffer as ArrayBuffer,
                                    },
                                ])
                            }
                        >
                            Upload fixture
                        </button>
                    </div>
                );
            },
        ),
    };
});

vi.mock("./FilePreviewPanel", () => ({
    FilePreviewPanel: ({
        file,
        startInEditMode,
        onClose,
        onDownload,
        onEditModeChange,
    }: {
        file: MockFileItem;
        startInEditMode?: boolean;
        onClose?: () => void;
        onDownload?: () => void;
        onEditModeChange?: (editing: boolean) => void;
    }) => (
        <aside aria-label={`Preview ${file.name}`}>
            <span>{startInEditMode ? `editing ${file.name}` : `previewing ${file.name}`}</span>
            <button type="button" onClick={() => onEditModeChange?.(true)}>
                Start edit mock
            </button>
            <button type="button" onClick={() => onEditModeChange?.(false)}>
                Stop edit mock
            </button>
            <button type="button" onClick={() => onDownload?.()}>
                Preview download {file.name}
            </button>
            <button type="button" onClick={() => onClose?.()}>
                Close preview mock
            </button>
        </aside>
    ),
}));

type MockFileItem = { id: string; name: string; type: string; size?: number };

type MockFileListProps = {
    files: MockFileItem[];
    selectedFiles?: Set<string>;
    canPaste?: boolean;
    onSelectFile?: (id: string, options?: { toggle?: boolean }) => void;
    onSelectFiles?: (ids: string[]) => void;
    onOpenFile?: (file: MockFileItem) => void;
    onEditFile?: (file: MockFileItem) => void;
    onDownloadFile?: (file: MockFileItem) => void;
    onDeleteFile?: (file: MockFileItem) => void;
    onCutSelected?: () => void;
    onCopySelected?: () => void;
    onPaste?: (targetDir?: string) => void;
    onMoveToFolder?: (paths: string[], folderPath: string) => void;
    onSortChange?: (field: "name" | "type" | "modified" | "size") => void;
};

function MockFileList({
    files,
    selectedFiles,
    canPaste,
    onSelectFile,
    onSelectFiles,
    onOpenFile,
    onEditFile,
    onDownloadFile,
    onDeleteFile,
    onCutSelected,
    onCopySelected,
    onPaste,
    onMoveToFolder,
    onSortChange,
    view,
}: MockFileListProps & { view: "grid" | "table" }) {
    return (
        <div data-testid={`${view}-view`}>
            {canPaste && <span>clipboard ready</span>}
            <button type="button" onClick={() => onSelectFiles?.(files.map((file) => file.id))}>
                Select all mock
            </button>
            <button type="button" onClick={() => onCutSelected?.()}>
                Cut selected
            </button>
            <button type="button" onClick={() => onCopySelected?.()}>
                Copy selected
            </button>
            <button type="button" disabled={!canPaste} onClick={() => onPaste?.()}>
                Paste selected
            </button>
            <button type="button" onClick={() => onSortChange?.("size")}>
                Sort by size
            </button>
            {files.map((file) => (
                <div key={file.id} data-testid={`${view}-item-${file.name}`}>
                    <button type="button" onClick={() => onSelectFile?.(file.id)}>
                        {file.name}
                    </button>
                    <button type="button" onClick={() => onSelectFile?.(file.id, { toggle: true })}>
                        Toggle {file.name}
                    </button>
                    {selectedFiles?.has(file.id) && <span>selected {file.name}</span>}
                    <button type="button" onClick={() => onOpenFile?.(file)}>
                        Open {file.name}
                    </button>
                    {file.type === "file" && (
                        <>
                            <button type="button" onClick={() => onEditFile?.(file)}>
                                Edit {file.name}
                            </button>
                            <button type="button" onClick={() => onDownloadFile?.(file)}>
                                Download {file.name}
                            </button>
                            <button type="button" onClick={() => onDeleteFile?.(file)}>
                                Delete {file.name}
                            </button>
                        </>
                    )}
                    {file.type === "folder" && (
                        <button type="button" onClick={() => onMoveToFolder?.(["/report.txt"], file.id)}>
                            Move report to {file.name}
                        </button>
                    )}
                </div>
            ))}
        </div>
    );
}

vi.mock("./FileGrid", () => ({
    FileGrid: (props: MockFileListProps) => <MockFileList {...props} view="grid" />,
}));

vi.mock("./FileTable", () => ({
    FileTable: (props: MockFileListProps) => <MockFileList {...props} view="table" />,
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

        renderFileBrowser();

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

        renderFileBrowser();

        await waitFor(() => {
            expect(screen.getByText("Access denied")).toBeInTheDocument();
            expect(
                screen.getByText("You don't have permission to view this location."),
            ).toBeInTheDocument();
        });
    });

    it("displays raw message for unknown errors", async () => {
        vi.mocked(listEntries).mockRejectedValue(new TauriApiError("Something went wrong", "UNKNOWN"));

        renderFileBrowser();

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
            withFileBrowserProviders(<FileBrowser sourceId="slow" storageName="Slow Storage" />),
        );

        await waitFor(() => {
            expect(listEntries).toHaveBeenCalledWith("slow", "/");
        });

        rerender(
            withFileBrowserProviders(<FileBrowser sourceId="fast" storageName="Fast Storage" />),
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
        renderFileBrowser();

        const search = await screen.findByPlaceholderText("Search...");
        fireEvent.keyDown(window, { key: "f", ctrlKey: true });
        expect(document.activeElement).toBe(search);
    });

    it("opens create folder dialog on Ctrl/Cmd+Shift+N and creates folder", async () => {
        renderFileBrowser();

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

describe("FileBrowser navigation, selection, and upload flows", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        vi.mocked(listEntries).mockImplementation((_sourceId, path) => {
            if (path === "/docs") {
                return Promise.resolve([
                    {
                        path: "/docs/readme.md",
                        name: "readme.md",
                        is_dir: false,
                        size: 40,
                        modified_at: null,
                    },
                ]);
            }

            return Promise.resolve([
                {
                    path: "/docs",
                    name: "docs",
                    is_dir: true,
                    size: 0,
                    modified_at: null,
                },
                {
                    path: "/report.txt",
                    name: "report.txt",
                    is_dir: false,
                    size: 12,
                    modified_at: null,
                },
                {
                    path: "/notes.md",
                    name: "notes.md",
                    is_dir: false,
                    size: 6,
                    modified_at: null,
                },
            ]);
        });
        vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("preview"));
        vi.mocked(writeFile).mockResolvedValue(undefined);
        vi.mocked(deletePath).mockResolvedValue(undefined);
        vi.mocked(transferEntries).mockResolvedValue(undefined);
    });

    it("filters files through the search input", async () => {
        renderFileBrowser();

        expect(await screen.findByRole("button", { name: "report.txt" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "notes.md" })).toBeInTheDocument();

        fireEvent.change(screen.getByPlaceholderText("Search..."), {
            target: { value: "notes" },
        });

        expect(screen.queryByRole("button", { name: "report.txt" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "notes.md" })).toBeInTheDocument();
    });

    it("switches to table view and applies size sorting", async () => {
        vi.mocked(listEntries).mockResolvedValue([
            {
                path: "/aaa-small.txt",
                name: "aaa-small.txt",
                is_dir: false,
                size: 1,
                modified_at: null,
            },
            {
                path: "/zzz-large.txt",
                name: "zzz-large.txt",
                is_dir: false,
                size: 100,
                modified_at: null,
            },
        ]);

        renderFileBrowser();

        expect(await screen.findByTestId("grid-view")).toBeInTheDocument();
        fireEvent.click(screen.getByTitle("Switch to list view"));
        const table = await screen.findByTestId("table-view");
        expect(
            within(table)
                .getAllByTestId(/table-item-/)
                .map((item) => item.getAttribute("data-testid")),
        ).toEqual(["table-item-aaa-small.txt", "table-item-zzz-large.txt"]);

        fireEvent.click(within(table).getByRole("button", { name: "Sort by size" }));
        expect(
            within(screen.getByTestId("table-view"))
                .getAllByTestId(/table-item-/)
                .map((item) => item.getAttribute("data-testid")),
        ).toEqual(["table-item-zzz-large.txt", "table-item-aaa-small.txt"]);
    });

    it("navigates into folders and updates the editable path footer", async () => {
        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "Open docs" }));

        await waitFor(() => {
            expect(listEntries).toHaveBeenCalledWith("test", "/docs");
        });
        expect(await screen.findByRole("button", { name: "readme.md" })).toBeInTheDocument();
        expect(screen.getByTitle("Click to edit path")).toHaveTextContent("/docs");
    });

    it("opens, edits, downloads, and closes the preview panel from file actions", async () => {
        const createObjectURL = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:preview-download");
        const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
        const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);

        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "Open report.txt" }));
        expect(await screen.findByLabelText("Preview report.txt")).toBeInTheDocument();
        expect(screen.getByText("previewing report.txt")).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "Start edit mock" }));
        expect(screen.getByText("editing report.txt")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Stop edit mock" }));
        expect(screen.getByText("previewing report.txt")).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "Preview download report.txt" }));
        await waitFor(() => {
            expect(readFile).toHaveBeenCalledWith("test", "/report.txt");
            expect(createObjectURL).toHaveBeenCalled();
            expect(revokeObjectURL).toHaveBeenCalledWith("blob:preview-download");
            expect(click).toHaveBeenCalled();
        });

        fireEvent.click(screen.getByRole("button", { name: "Close preview mock" }));
        expect(screen.queryByLabelText("Preview report.txt")).not.toBeInTheDocument();

        createObjectURL.mockRestore();
        revokeObjectURL.mockRestore();
        click.mockRestore();
    });

    it("deletes selected files only after confirming the keyboard delete dialog", async () => {
        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "report.txt" }));
        expect(await screen.findByText("selected report.txt")).toBeInTheDocument();
        fireEvent.keyDown(window, { key: "Delete" });

        expect(await screen.findByText("Delete selected items?")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Delete" }));

        await waitFor(() => {
            expect(deletePath).toHaveBeenCalledWith("test", "/report.txt");
            expect(listEntries).toHaveBeenCalledWith("test", "/");
        });
    });

    it("requires confirmation before deleting from a visible file action", async () => {
        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "Delete report.txt" }));

        expect(await screen.findByText("Delete report.txt?")).toBeInTheDocument();
        expect(deletePath).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(deletePath).not.toHaveBeenCalled();

        fireEvent.click(screen.getByRole("button", { name: "Delete report.txt" }));
        fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

        await waitFor(() => {
            expect(deletePath).toHaveBeenCalledWith("test", "/report.txt");
        });
    });

    it("shows visible progress while deletion is in progress", async () => {
        const pendingDelete = deferred<void>();
        vi.mocked(deletePath).mockReturnValueOnce(pendingDelete.promise);
        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "report.txt" }));
        fireEvent.keyDown(window, { key: "Delete" });
        fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

        expect(await screen.findByLabelText("Deletion in progress")).toBeInTheDocument();
        expect(screen.getByText("Deleting 1 item")).toBeInTheDocument();
        expect(screen.getByText(/Removing report\.txt/)).toBeInTheDocument();

        pendingDelete.resolve();
        await waitFor(() => {
            expect(screen.queryByLabelText("Deletion in progress")).not.toBeInTheDocument();
        });
    });

    it("uploads from the upload control and external drop handler", async () => {
        const { container } = renderFileBrowser();

        await screen.findByTestId("grid-view");
        fireEvent.click(screen.getByRole("button", { name: "Upload fixture" }));

        await waitFor(() => {
            expect(writeFile).toHaveBeenCalledWith("test", "/fixture.txt", expect.any(Uint8Array));
            expect(listEntries).toHaveBeenCalledWith("test", "/");
        });

        const droppedFile = {
            name: "drop.txt",
            arrayBuffer: async () => new TextEncoder().encode("drop").buffer,
        };
        const dropEvent = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(dropEvent, "dataTransfer", {
            value: {
                files: [],
                items: [
                    {
                        kind: "file",
                        getAsFile: () => droppedFile,
                    },
                ],
            },
        });
        container.firstElementChild!.dispatchEvent(dropEvent);

        await waitFor(() => {
            expect(writeFile).toHaveBeenCalledWith("test", "/drop.txt", expect.any(Uint8Array));
        });

        const fallbackFile = {
            name: "fallback.txt",
            arrayBuffer: async () => new TextEncoder().encode("fallback").buffer,
        };
        const fallbackDrop = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(fallbackDrop, "dataTransfer", {
            value: {
                files: [fallbackFile],
                items: [],
            },
        });
        container.firstElementChild!.dispatchEvent(fallbackDrop);

        await waitFor(() => {
            expect(writeFile).toHaveBeenCalledWith("test", "/fallback.txt", expect.any(Uint8Array));
        });

        const nestedFile = {
            name: "nested.txt",
            arrayBuffer: async () => new TextEncoder().encode("nested").buffer,
        };
        const fileEntry = {
            name: "nested.txt",
            isFile: true,
            isDirectory: false,
            file: (success: (file: unknown) => void) => success(nestedFile),
        };
        let readCount = 0;
        const folderEntry = {
            name: "folder",
            isFile: false,
            isDirectory: true,
            createReader: () => ({
                readEntries: (success: (entries: unknown[]) => void) => {
                    readCount += 1;
                    success(readCount === 1 ? [fileEntry] : []);
                },
            }),
        };
        const folderDrop = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(folderDrop, "dataTransfer", {
            value: {
                files: [],
                items: [
                    {
                        kind: "file",
                        webkitGetAsEntry: () => folderEntry,
                    },
                ],
            },
        });
        container.firstElementChild!.dispatchEvent(folderDrop);

        await waitFor(() => {
            expect(writeFile).toHaveBeenCalledWith("test", "/folder/nested.txt", expect.any(Uint8Array));
        });

        const ignoredItemDrop = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(ignoredItemDrop, "dataTransfer", {
            value: {
                files: [
                    {
                        name: "fallback-after-ignored.txt",
                        arrayBuffer: async () => new TextEncoder().encode("ignored").buffer,
                    },
                ],
                items: [{ kind: "string" }],
            },
        });
        container.firstElementChild!.dispatchEvent(ignoredItemDrop);

        await waitFor(() => {
            expect(writeFile).toHaveBeenCalledWith(
                "test",
                "/fallback-after-ignored.txt",
                expect.any(Uint8Array),
            );
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
        vi.mocked(listEntriesRecursive).mockResolvedValue([
            {
                path: "/report.txt",
                name: "report.txt",
                is_dir: false,
                size: 12,
                modified_at: null,
            },
        ]);
        vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("hello"));
        vi.mocked(planTransferEntries).mockResolvedValue({
            operation: "copy",
            conflictPolicy: "fail",
            entries: [],
            summary: {
                create: 1,
                overwrite: 0,
                skip: 0,
                rename: 0,
                noop: 0,
                conflict: 0,
                totalItems: 1,
                totalBytes: 12,
            },
        });
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
                expect.stringMatching(/^transfer-/),
            );
        });
    });

    it("copies selected files to the opposite split pane", async () => {
        render(
            withFileBrowserProviders(
                <FileBrowser
                    sourceId="left"
                    storageName="Left Storage"
                    paneTransferTarget={{
                        sourceId: "right",
                        storageName: "Right Storage",
                        currentPath: "/incoming",
                        direction: "right",
                    }}
                />,
            ),
        );

        fireEvent.click(await screen.findByText("report.txt"));
        await screen.findByText("selected report.txt");
        fireEvent.click(screen.getByRole("button", { name: "Copy →" }));

        await waitFor(() => {
            expect(transferEntries).toHaveBeenCalledWith(
                "left",
                "right",
                ["/report.txt"],
                "/incoming",
                "copy",
                "fail",
                expect.stringMatching(/^transfer-/),
            );
        });
    });

    it("lets users overwrite paste conflicts", async () => {
        vi.mocked(transferEntries)
            .mockRejectedValueOnce(new TauriApiError("already exists", "ALREADY_EXISTS"))
            .mockResolvedValueOnce(undefined);

        renderFileBrowser();

        fireEvent.click(await screen.findByText("report.txt"));
        fireEvent.click(screen.getByRole("button", { name: "Copy selected" }));
        await screen.findByText("clipboard ready");
        fireEvent.click(screen.getByRole("button", { name: "Paste selected" }));

        expect(await screen.findByText("Item already exists")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Overwrite" }));

        await waitFor(() => {
            expect(transferEntries).toHaveBeenLastCalledWith(
                "test",
                "test",
                ["/report.txt"],
                "/",
                "copy",
                "overwrite",
                expect.stringMatching(/^transfer-/),
            );
        });
    });

    it("lets users keep both conflicting transfers", async () => {
        vi.mocked(transferEntries)
            .mockRejectedValueOnce(new TauriApiError("already exists", "ALREADY_EXISTS"))
            .mockResolvedValueOnce(undefined);

        renderFileBrowser();

        fireEvent.click(await screen.findByText("report.txt"));
        fireEvent.click(screen.getByRole("button", { name: "Copy selected" }));
        await screen.findByText("clipboard ready");
        fireEvent.click(screen.getByRole("button", { name: "Paste selected" }));

        expect(await screen.findByText("Item already exists")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Keep both" }));

        await waitFor(() => {
            expect(transferEntries).toHaveBeenLastCalledWith(
                "test",
                "test",
                ["/report.txt"],
                "/",
                "copy",
                "rename",
                expect.stringMatching(/^transfer-/),
            );
        });
    });

    it("lets users discard conflicting move-into-folder transfers", async () => {
        vi.mocked(listEntries).mockResolvedValue([
            {
                path: "/folder",
                name: "folder",
                is_dir: true,
                size: 0,
                modified_at: null,
            },
            {
                path: "/report.txt",
                name: "report.txt",
                is_dir: false,
                size: 12,
                modified_at: null,
            },
        ]);
        vi.mocked(transferEntries)
            .mockRejectedValueOnce(new TauriApiError("already exists", "ALREADY_EXISTS"))
            .mockResolvedValueOnce(undefined);

        renderFileBrowser();

        fireEvent.click(await screen.findByRole("button", { name: "Move report to folder" }));

        expect(await screen.findByText("Item already exists")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Discard" }));

        await waitFor(() => {
            expect(transferEntries).toHaveBeenLastCalledWith(
                "test",
                "test",
                ["/report.txt"],
                "/folder",
                "move",
                "skip",
                expect.stringMatching(/^transfer-/),
            );
        });
    });
});
