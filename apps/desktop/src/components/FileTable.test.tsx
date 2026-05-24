import { render, screen, fireEvent } from "@testing-library/react";
import { FileTable } from "./FileTable";
import { FileItem } from "@/types/storage";
import { vi, describe, it, expect } from "vitest";

const mockFiles: FileItem[] = [
    {
        id: "/folder1",
        name: "folder1",
        type: "folder",
        modified: new Date("2023-01-01"),
        size: 0,
    },
    {
        id: "/file1.txt",
        name: "file1.txt",
        type: "file",
        extension: "txt",
        modified: new Date("2023-01-02"),
        size: 1024,
    },
];

// Mock @tanstack/react-virtual
vi.mock("@tanstack/react-virtual", () => ({
    useVirtualizer: vi.fn(({ count }: { count: number }) => ({
        getVirtualItems: () =>
            Array.from({ length: count }, (_, index) => ({
                index,
                start: index * 53,
                end: (index + 1) * 53,
                size: 53,
                measureElement: vi.fn(),
            })),
        getTotalSize: () => count * 53,
    })),
}));

describe("FileTable", () => {
    it("renders list of files", () => {
        render(
            <FileTable
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        expect(screen.getByText("folder1")).toBeInTheDocument();
        expect(screen.getByText("file1.txt")).toBeInTheDocument();
    });

    it("calls onSelectFile when clicked", () => {
        const onSelectFile = vi.fn();
        render(
            <FileTable
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
            <FileTable
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onOpenFile={onOpenFile}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        // Double click the row (or a cell in the row)
        fireEvent.doubleClick(screen.getByText("folder1"));
        expect(onOpenFile).toHaveBeenCalledWith(mockFiles[0]);
    });

    it("supports ctrl-click selection and sort header callbacks", () => {
        const onSelectFile = vi.fn();
        const onSortChange = vi.fn();
        render(
            <FileTable
                sourceId="test"
                files={mockFiles}
                selectedFiles={new Set(["/file1.txt"])}
                onSelectFile={onSelectFile}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
                sortField="name"
                sortDirection="desc"
                onSortChange={onSortChange}
            />
        );

        expect(screen.getByText(/Name ▼/)).toBeInTheDocument();
        fireEvent.click(screen.getByText("file1.txt"), { ctrlKey: true });
        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt", { toggle: true });

        fireEvent.click(screen.getByText(/Type/));
        fireEvent.click(screen.getByText(/Modified/));
        fireEvent.click(screen.getByText(/Size/));
        expect(onSortChange).toHaveBeenNthCalledWith(1, "type");
        expect(onSortChange).toHaveBeenNthCalledWith(2, "modified");
        expect(onSortChange).toHaveBeenNthCalledWith(3, "size");
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
            <FileTable
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

    it("formats size and modified-date branches", () => {
        const now = new Date();
        const oneDayAgo = new Date(now.getTime() - 24 * 60 * 60 * 1000);
        const threeDaysAgo = new Date(now.getTime() - 3 * 24 * 60 * 60 * 1000);
        const old = new Date("2022-02-03T12:00:00Z");
        const files: FileItem[] = [
            { id: "/bytes.bin", name: "bytes.bin", type: "file", size: 512, modified: now, extension: "bin" },
            { id: "/yesterday.bin", name: "yesterday.bin", type: "file", size: 1024, modified: oneDayAgo, extension: "bin" },
            { id: "/days.bin", name: "days.bin", type: "file", size: 2 * 1024 * 1024, modified: threeDaysAgo, extension: "bin" },
            { id: "/gb.bin", name: "gb.bin", type: "file", size: 3 * 1024 * 1024 * 1024, modified: old, extension: "bin" },
            { id: "/unknown", name: "unknown", type: "file", modified: null },
        ];

        render(
            <FileTable
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
        expect(screen.getByText("Yesterday")).toBeInTheDocument();
        expect(screen.getByText("3 days ago")).toBeInTheDocument();
        expect(screen.getAllByText("-").length).toBeGreaterThanOrEqual(2);
    });

    it("writes internal drag payloads and selects unselected dragged files", () => {
        const onSelectFile = vi.fn();
        const setData = vi.fn();
        const dataTransfer = {
            setData,
            effectAllowed: "",
        } as unknown as DataTransfer;

        render(
            <FileTable
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
            <FileTable
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
            <FileTable
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
});
