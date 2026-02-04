import { render, screen, fireEvent } from "@testing-library/react";
import { FileGrid } from "./FileGrid";
import { describe, it, expect, vi } from "vitest";
import { FileItem } from "@/types/storage";


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
    useVirtualizer: vi.fn().mockReturnValue({
        getVirtualItems: () => [
            { index: 0, start: 0, size: 100, measureElement: vi.fn() },
            { index: 1, start: 100, size: 100, measureElement: vi.fn() },
        ],
        getTotalSize: () => 200,
    }),
}));

describe("FileGrid", () => {

    it("renders list of files", () => {
        render(
            <FileGrid
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
});
