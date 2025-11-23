import { render, screen, waitFor } from "@testing-library/react";
import { FileBrowser } from "./FileBrowser";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { listEntries, TauriApiError } from "@/lib/api";

// Mock the api module
vi.mock("@/lib/api", () => ({
    listEntries: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    deletePath: vi.fn(),
    TauriApiError: class extends Error {
        code: string;
        constructor(message: string, code: string) {
            super(message);
            this.code = code;
        }
    },
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
        (listEntries as any).mockRejectedValue(new TauriApiError("Raw error", "NOT_FOUND"));

        render(<FileBrowser sourceId="test" storageName="Test Storage" />);

        await waitFor(() => {
            expect(screen.getByText("The requested folder does not exist.")).toBeInTheDocument();
        });
    });

    it("displays user-friendly message for PERMISSION_DENIED error", async () => {
        (listEntries as any).mockRejectedValue(new TauriApiError("Raw error", "PERMISSION_DENIED"));

        render(<FileBrowser sourceId="test" storageName="Test Storage" />);

        await waitFor(() => {
            expect(screen.getByText("Access denied. You do not have permission to view this folder.")).toBeInTheDocument();
        });
    });

    it("displays raw message for unknown errors", async () => {
        (listEntries as any).mockRejectedValue(new TauriApiError("Something went wrong", "UNKNOWN"));

        render(<FileBrowser sourceId="test" storageName="Test Storage" />);

        await waitFor(() => {
            expect(screen.getByText("Something went wrong")).toBeInTheDocument();
        });
    });
});
