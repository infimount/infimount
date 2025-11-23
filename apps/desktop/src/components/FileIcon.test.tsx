import { getFileIcon, getFileColor } from "./FileIcon";
import { describe, it, expect } from "vitest";
import { Folder, FileImage, FileText, File } from "lucide-react";
import { FileItem } from "@/types/storage";

describe("FileIcon Utilities", () => {
    it("returns Folder icon for folder type", () => {
        const item: FileItem = { id: "1", name: "docs", type: "folder", modified: null, size: 0 };
        expect(getFileIcon(item)).toBe(Folder);
    });

    it("returns FileImage icon for image extensions", () => {
        const item: FileItem = { id: "2", name: "img.png", type: "file", extension: "png", modified: null, size: 1024 };
        expect(getFileIcon(item)).toBe(FileImage);
    });

    it("returns FileText icon for text extensions", () => {
        const item: FileItem = { id: "3", name: "note.txt", type: "file", extension: "txt", modified: null, size: 1024 };
        expect(getFileIcon(item)).toBe(FileText);
    });

    it("returns default File icon for unknown extensions", () => {
        const item: FileItem = { id: "4", name: "unknown.xyz", type: "file", extension: "xyz", modified: null, size: 1024 };
        expect(getFileIcon(item)).toBe(File);
    });

    it("returns correct color class for folders", () => {
        const item: FileItem = { id: "1", name: "docs", type: "folder", modified: null, size: 0 };
        expect(getFileColor(item)).toBe("text-primary");
    });

    it("returns correct color class for images", () => {
        const item: FileItem = { id: "2", name: "img.png", type: "file", extension: "png", modified: null, size: 1024 };
        expect(getFileColor(item)).toBe("text-pink-500");
    });
});
