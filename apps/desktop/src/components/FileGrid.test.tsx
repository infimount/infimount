import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { FileGrid } from "./FileGrid";
import { describe, it, expect, vi } from "vitest";
import { FileItem } from "@/types/storage";


const rect = (left: number, top: number, width: number, height: number) => ({
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    x: left,
    y: top,
    toJSON: () => ({}),
}) as DOMRect;

const mockFiles: FileItem[] = [
    {
        id: "/folder1",
        name: "folder1",
        type: "folder",
        modified: new Date(),
        size: 0,
    },
    {
        id: "/file1.txt",
        name: "file1.txt",
        type: "file",
        modified: new Date(),
        size: 1024,
        extension: "txt",
    },
];

// Mock @tanstack/react-virtual
vi.mock("@tanstack/react-virtual", () => ({
    useVirtualizer: vi.fn(({ count }: { count: number }) => ({
        getVirtualItems: () =>
            Array.from({ length: count }, (_, index) => ({
                index,
                start: index * 112,
                end: (index + 1) * 112,
                size: 112,
                measureElement: vi.fn(),
            })),
        getTotalSize: () => count * 112,
    })),
}));

describe("FileGrid", () => {

    it("renders list of files", () => {
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        // With the mock, we expect 2 items to be rendered.
        // The mock returns index 0 and 1.
        // files[0] is folder1, files[1] is file1.txt
        expect(screen.getByText("folder1")).toBeInTheDocument();
        expect(screen.getByText("file1.txt")).toBeInTheDocument();
    });

    it("calls onSelectFile when clicked", () => {
        const onSelectFile = vi.fn();
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={onSelectFile}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        fireEvent.click(screen.getByText("file1.txt"));
        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt");
    });

    it("calls onOpenFile when double clicked", () => {
        const onOpenFile = vi.fn();
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onOpenFile={onOpenFile}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        fireEvent.doubleClick(screen.getByText("folder1"));
        expect(onOpenFile).toHaveBeenCalledWith(mockFiles[0]);
    });

    it("supports ctrl-click selection", () => {
        const onSelectFile = vi.fn();
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set(["/file1.txt"])}
                onSelectFile={onSelectFile}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        fireEvent.click(screen.getByText("file1.txt"), { metaKey: true });
        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt", { toggle: true });
    });

    it("supports roving keyboard navigation", async () => {
        const onSelectFile = vi.fn();
        const onOpenFile = vi.fn();
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={onSelectFile}
                onOpenFile={onOpenFile}
            />
        );

        const folder = screen.getByRole("option", { name: "folder1" });
        const file = screen.getByRole("option", { name: "file1.txt" });

        expect(folder).toHaveAttribute("tabindex", "0");
        folder.focus();
        fireEvent.keyDown(folder, { key: "ArrowRight" });

        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt");
        await waitFor(() => expect(file).toHaveFocus());

        fireEvent.keyDown(file, { key: "Enter" });
        expect(onOpenFile).toHaveBeenCalledWith(mockFiles[1]);

        fireEvent.keyDown(file, { key: " " });
        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt", { toggle: true });
    });

    it("exposes file actions through the context menu", async () => {
        const onOpenFile = vi.fn();
        const onEditFile = vi.fn();
        const onDownloadFile = vi.fn();
        const onDeleteFile = vi.fn();
        const onCutSelected = vi.fn();
        const onCopySelected = vi.fn();
        const onPaste = vi.fn();
        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set(["/file1.txt"])}
                onSelectFile={() => { }}
                onOpenFile={onOpenFile}
                onEditFile={onEditFile}
                onDownloadFile={onDownloadFile}
                onDeleteFile={onDeleteFile}
                onCutSelected={onCutSelected}
                onCopySelected={onCopySelected}
                canPaste
                onPaste={onPaste}
            />
        );

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Preview"));
        expect(onOpenFile).toHaveBeenCalledWith(mockFiles[1]);

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Download"));
        expect(onDownloadFile).toHaveBeenCalledWith(mockFiles[1]);

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Edit"));
        expect(onEditFile).toHaveBeenCalledWith(mockFiles[1]);

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Delete"));
        expect(onDeleteFile).toHaveBeenCalledWith(mockFiles[1]);

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Cut"));
        expect(onCutSelected).toHaveBeenCalled();

        fireEvent.contextMenu(screen.getByText("file1.txt"));
        fireEvent.click(await screen.findByText("Copy"));
        expect(onCopySelected).toHaveBeenCalled();

        fireEvent.contextMenu(screen.getByText("folder1"));
        fireEvent.click(await screen.findByText("Paste into folder"));
        expect(onPaste).toHaveBeenCalledWith("/folder1");
    });

    it("formats file sizes and truncates long names", () => {
        const files: FileItem[] = [
            { id: "/bytes.bin", name: "bytes.bin", type: "file", size: 512, modified: new Date(), extension: "bin" },
            { id: "/kb.bin", name: "kb.bin", type: "file", size: 1024, modified: new Date(), extension: "bin" },
            { id: "/mb.bin", name: "mb.bin", type: "file", size: 2 * 1024 * 1024, modified: new Date(), extension: "bin" },
            { id: "/gb.bin", name: "gb.bin", type: "file", size: 3 * 1024 * 1024 * 1024, modified: new Date(), extension: "bin" },
            {
                id: "/long.json",
                name: "very-long-file-name-with-many-characters.json",
                type: "file",
                size: 0,
                modified: new Date(),
                extension: "json",
            },
        ];

        render(
            <FileGrid
                sourceId="test"
                files={files}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
            />
        );

        expect(screen.getByText("512 B")).toBeInTheDocument();
        expect(screen.getByText("1.0 KB")).toBeInTheDocument();
        expect(screen.getByText("2.0 MB")).toBeInTheDocument();
        expect(screen.getByText("3.0 GB")).toBeInTheDocument();
        expect(screen.getByText("very-long...json")).toBeInTheDocument();
    });

    it("writes internal drag payloads and selects unselected dragged files", () => {
        const onSelectFile = vi.fn();
        const setData = vi.fn();
        const dataTransfer = {
            setData,
            effectAllowed: "",
        } as unknown as DataTransfer;

        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={onSelectFile}
            />
        );

        fireEvent.dragStart(screen.getByText("file1.txt"), { dataTransfer });

        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt");
        expect(setData).toHaveBeenCalledWith(
            "application/x-infimount-transfer",
            JSON.stringify({
                kind: "infimount-transfer",
                fromSourceId: "test",
                paths: ["/file1.txt"],
                operation: "copy",
            }),
        );
        expect(dataTransfer.effectAllowed).toBe("copyMove");
    });

    it("uses the current selection when dragging an already-selected item", () => {
        const onSelectFile = vi.fn();
        const setData = vi.fn();
        const dataTransfer = {
            setData,
            effectAllowed: "",
        } as unknown as DataTransfer;

        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set(["/folder1", "/file1.txt"])}
                onSelectFile={onSelectFile}
            />
        );

        fireEvent.dragStart(screen.getByText("file1.txt"), { dataTransfer });

        expect(onSelectFile).not.toHaveBeenCalled();
        expect(setData).toHaveBeenCalledWith(
            "application/x-infimount-transfer",
            JSON.stringify({
                kind: "infimount-transfer",
                fromSourceId: "test",
                paths: ["/folder1", "/file1.txt"],
                operation: "copy",
            }),
        );
    });

    it("ignores external, malformed, and cross-source folder drops", () => {
        const onMoveToFolder = vi.fn();
        const malformedTransfer = {
            types: ["text/plain"],
            files: [],
            items: [],
            dropEffect: "copy",
            getData: vi.fn(() => "not-json"),
            setData: vi.fn(),
        } as unknown as DataTransfer;
        const crossSourceTransfer = {
            types: ["application/x-infimount-transfer"],
            files: [],
            items: [],
            dropEffect: "copy",
            getData: vi.fn(() => JSON.stringify({
                kind: "infimount-transfer",
                fromSourceId: "other",
                paths: ["/file1.txt"],
            })),
            setData: vi.fn(),
        } as unknown as DataTransfer;
        const externalTransfer = {
            types: ["Files"],
            files: [{ name: "local.txt" }],
            items: [{ kind: "file" }],
            dropEffect: "copy",
            getData: vi.fn(() => ""),
            setData: vi.fn(),
        } as unknown as DataTransfer;

        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onMoveToFolder={onMoveToFolder}
            />
        );

        fireEvent.dragOver(screen.getByText("folder1"), { dataTransfer: externalTransfer });
        fireEvent.drop(screen.getByText("folder1"), { dataTransfer: malformedTransfer });
        fireEvent.drop(screen.getByText("folder1"), { dataTransfer: crossSourceTransfer });

        expect(onMoveToFolder).not.toHaveBeenCalled();
    });

    it("moves internally dragged files onto folders", () => {
        const onMoveToFolder = vi.fn();
        const payload = JSON.stringify({
            kind: "infimount-transfer",
            fromSourceId: "test",
            paths: ["/file1.txt"],
            operation: "move",
        });
        const dataTransfer = {
            types: ["application/x-infimount-transfer"],
            files: [],
            items: [],
            dropEffect: "copy",
            getData: vi.fn((type: string) =>
                type === "application/x-infimount-transfer" || type === "text/plain" ? payload : "",
            ),
            setData: vi.fn(),
        } as unknown as DataTransfer;

        render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
                onMoveToFolder={onMoveToFolder}
            />
        );

        fireEvent.dragOver(screen.getByText("folder1"), { dataTransfer });
        fireEvent.drop(screen.getByText("folder1"), { dataTransfer });

        expect(onMoveToFolder).toHaveBeenCalledWith(["/file1.txt"], "/folder1");
    });

    it("drag-selects grid cards from the empty grid background", async () => {
        const onSelectFiles = vi.fn();
        const onClearSelection = vi.fn();
        const { container } = render(
            <FileGrid
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onSelectFiles={onSelectFiles}
                onClearSelection={onClearSelection}
            />
        );

        const scrollContainer = container.querySelector(".overflow-y-auto") as HTMLDivElement;
        Object.defineProperty(scrollContainer, "getBoundingClientRect", {
            value: () => rect(0, 0, 600, 400),
        });

        const cards = Array.from(
            container.querySelectorAll<HTMLDivElement>('[data-infimount-file-item="true"]'),
        );
        Object.defineProperty(cards[0], "getBoundingClientRect", {
            value: () => rect(10, 10, 96, 96),
        });
        Object.defineProperty(cards[1], "getBoundingClientRect", {
            value: () => rect(120, 10, 96, 96),
        });

        fireEvent.mouseDown(scrollContainer, { button: 0, clientX: 0, clientY: 0 });
        expect(onClearSelection).toHaveBeenCalledTimes(1);

        fireEvent.mouseMove(scrollContainer, { buttons: 1, clientX: 240, clientY: 120 });

        await waitFor(() => expect(onSelectFiles).toHaveBeenLastCalledWith(["/folder1", "/file1.txt"]));

        fireEvent.mouseUp(scrollContainer);
        fireEvent.mouseLeave(scrollContainer, { buttons: 1 });
    });
});
