import { render, waitFor } from "@testing-library/react";
import { FileTypeIcon, getFileIconKey, getFileIconPath } from "./FileIcon";
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

    it("detects known dotfile names without an extension", () => {
        const item: FileItem = { id: "5", name: ".gitignore", type: "file", modified: null, size: 0 };
        expect(getFileIconKey(item)).toBe("gitignore");
    });

    it("loads themed icon paths and caches repeated lookups", async () => {
        const item: FileItem = { id: "6", name: "photo.PNG", type: "file", extension: "PNG", modified: null, size: 1 };

        const first = await getFileIconPath(item, "vivid");
        const second = await getFileIconPath(item, "vivid");
        const otherThemes = await Promise.all([
            getFileIconPath(item, "classic"),
            getFileIconPath(item, "modern"),
            getFileIconPath(item, "square"),
        ]);

        expect(first).toBeTruthy();
        expect(second).toBe(first);
        expect(otherThemes.every(Boolean)).toBe(true);
    });

    it("renders an image icon and updates after the async theme map loads", async () => {
        const item: FileItem = { id: "7", name: "README.md", type: "file", extension: "md", modified: null, size: 1 };

        const { container } = render(<FileTypeIcon item={item} className="h-4 w-4" />);
        const image = container.querySelector("img")!;
        const initialSrc = image.getAttribute("src");
        expect(image).toHaveAttribute("aria-hidden", "true");

        await waitFor(() => {
            expect(image.getAttribute("src")).toBeTruthy();
            expect(image.getAttribute("src")).not.toBe(initialSrc);
        });
    });
});
