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
            { index: 0, start: 0, size: 53, measureElement: vi.fn() },
            { index: 1, start: 53, size: 53, measureElement: vi.fn() },
        ],
        getTotalSize: () => 106,
    }),
}));

describe("FileTable", () => {
    it("renders list of files", () => {
        render(
            <FileTable
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onSelectAll={() => { }}
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
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={onSelectFile}
                onSelectAll={() => { }}
                onOpenFile={() => { }}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        // Find checkboxes. Index 0 is header, 1 is folder1, 2 is file1.txt
        const checkboxes = screen.getAllByRole("checkbox");
        fireEvent.click(checkboxes[2]);
        expect(onSelectFile).toHaveBeenCalledWith("/file1.txt");
    });

    it("calls onOpenFile when double clicked", () => {
        const onOpenFile = vi.fn();
        render(
            <FileTable
                files={mockFiles}
                selectedFiles={new Set()}
                onSelectFile={() => { }}
                onSelectAll={() => { }}
                onOpenFile={onOpenFile}
                onDownloadFile={() => { }}
                onDeleteFile={() => { }}
            />
        );

        // Double click the row (or a cell in the row)
        fireEvent.doubleClick(screen.getByText("folder1"));
        expect(onOpenFile).toHaveBeenCalledWith(mockFiles[0]);
    });
});
