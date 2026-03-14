import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { StorageSidebar } from "./StorageSidebar";
import type { StorageConfig } from "@/types/storage";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("0.1.0"),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/lib/api", () => ({
  transferEntries: vi.fn(),
  TauriApiError: class extends Error {
    code: string;
    constructor(message: string, code = "UNKNOWN") {
      super(message);
      this.code = code;
    }
  },
}));

const toast = vi.fn();
vi.mock("@/hooks/use-toast", () => ({
  useToast: () => ({ toast }),
}));

const storages: StorageConfig[] = [
  {
    id: "local",
    name: "Local Docs",
    backend: "local",
    type: "local-fs",
    connected: true,
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    config: {},
  },
  {
    id: "gcs",
    name: "Google Bucket",
    backend: "gcs",
    type: "gcs",
    connected: true,
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    config: {},
  },
];

describe("StorageSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("filters storages from the inline search input", async () => {
    render(
      <StorageSidebar
        storages={storages}
        selectedStorage="local"
        onSelectStorage={() => undefined}
        onAddStorage={() => undefined}
        onEditStorage={() => undefined}
        onDeleteStorage={() => undefined}
        onRefreshStorage={() => undefined}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("v0.1.0")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTitle("Search storages"));
    fireEvent.change(screen.getByPlaceholderText("Search storages..."), {
      target: { value: "google" },
    });

    expect(screen.queryByText("Local Docs")).not.toBeInTheDocument();
    expect(screen.getByText("Google Bucket")).toBeInTheDocument();
  });

  it("selects a storage when clicked", async () => {
    const onSelectStorage = vi.fn();

    render(
      <StorageSidebar
        storages={storages}
        selectedStorage="local"
        onSelectStorage={onSelectStorage}
        onAddStorage={() => undefined}
        onEditStorage={() => undefined}
        onDeleteStorage={() => undefined}
        onRefreshStorage={() => undefined}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("v0.1.0")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /Google Bucket/i }));
    expect(onSelectStorage).toHaveBeenCalledWith("gcs");
  });
});
