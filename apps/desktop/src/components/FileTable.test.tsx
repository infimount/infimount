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
    useVirtualizer: vi.fn().mockReturnValue({
        getVirtualItems: () => [
            { index: 0, start: 0, end: 53, size: 53, measureElement: vi.fn() },
            { index: 1, start: 53, end: 106, size: 53, measureElement: vi.fn() },
        ],
        getTotalSize: () => 106,
    }),
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
