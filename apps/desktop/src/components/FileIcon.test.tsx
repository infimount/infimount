import { getFileIconKey } from "./FileIcon";
import { describe, it, expect } from "vitest";
import { FileItem } from "@/types/storage";

describe("FileIcon Utilities", () => {
    it("returns folder key for folder type", () => {
        const item: FileItem = { id: "1", name: "docs", type: "folder", modified: null, size: 0 };
        expect(getFileIconKey(item)).toBe("folder");
    });

    it("returns extension key for known types", () => {
        const item: FileItem = { id: "2", name: "img.png", type: "file", extension: "png", modified: null, size: 1024 };
        expect(getFileIconKey(item)).toBe("png");
    });

    it("returns extension key for text types", () => {
        const item: FileItem = { id: "3", name: "note.txt", type: "file", extension: "txt", modified: null, size: 1024 };
        expect(getFileIconKey(item)).toBe("txt");
    });

    it("returns default key for unknown extensions", () => {
        const item: FileItem = { id: "4", name: "unknown.xyz", type: "file", extension: "xyz", modified: null, size: 1024 };
        expect(getFileIconKey(item)).toBe("default");
    });
});
