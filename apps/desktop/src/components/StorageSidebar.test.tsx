import type { ComponentProps } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { check } from "@tauri-apps/plugin-updater";
import { TauriApiError, transferEntries } from "@/lib/api";
import { StorageSidebar } from "./StorageSidebar";
import type { McpStoragePolicy, StorageConfig } from "@/types/storage";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("0.1.0"),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/lib/api", () => ({
  connectOAuthStorage: vi.fn(),
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

const mcpPolicy: McpStoragePolicy = {
  default_access: "read_write",
  allowed_paths: [],
  denied_paths: [],
  confirmation_rules: {
    require_for_write: true,
    require_for_overwrite: true,
    require_for_delete: true,
    require_for_version_delete: true,
    require_for_presign: true,
    require_for_cross_storage_copy: true,
  },
};

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
    mcpPolicy,
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
    mcpPolicy,
  },
];

function renderSidebar(overrides: Partial<ComponentProps<typeof StorageSidebar>> = {}) {
  return render(
    <StorageSidebar
      storages={storages}
      selectedStorage="local"
      onSelectStorage={() => undefined}
      onAddStorage={() => undefined}
      onEditStorage={() => undefined}
      onDeleteStorage={() => undefined}
      onRefreshStorage={() => undefined}
      {...overrides}
    />,
  );
}

function internalTransferDataTransfer(payload: unknown): DataTransfer {
  const raw = JSON.stringify(payload);
  return {
    types: ["application/x-infimount-transfer"],
    files: [],
    items: [],
    dropEffect: "copy",
    getData: vi.fn((type: string) =>
      type === "application/x-infimount-transfer" || type === "text/plain" ? raw : "",
    ),
  } as unknown as DataTransfer;
}

describe("StorageSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(check).mockResolvedValue(null);
  });

  it("filters storages from the inline search input", async () => {
    renderSidebar();

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

  it("shows empty and loading states", () => {
    const { rerender } = renderSidebar({ storages: [], selectedStorage: null, isLoading: true });

    expect(screen.getByText("Loading storages…")).toBeInTheDocument();

    rerender(
      <StorageSidebar
        storages={[]}
        selectedStorage={null}
        onSelectStorage={() => undefined}
        onAddStorage={() => undefined}
        onEditStorage={() => undefined}
        onDeleteStorage={() => undefined}
        onRefreshStorage={() => undefined}
      />,
    );

    expect(screen.getByText("No storages found.")).toBeInTheDocument();
  });

  it("selects a storage when clicked or activated by keyboard", async () => {
    const onSelectStorage = vi.fn();

    renderSidebar({ onSelectStorage });

    await waitFor(() => {
      expect(screen.getByText("v0.1.0")).toBeInTheDocument();
    });

    const google = screen.getByRole("button", { name: /Google Bucket/i });
    fireEvent.click(google);
    fireEvent.keyDown(google, { key: "Enter" });
    fireEvent.keyDown(google, { key: " " });

    expect(onSelectStorage).toHaveBeenNthCalledWith(1, "gcs");
    expect(onSelectStorage).toHaveBeenNthCalledWith(2, "gcs");
    expect(onSelectStorage).toHaveBeenNthCalledWith(3, "gcs");
  });

  it("routes top menu and storage context menu actions", async () => {
    const handlers = {
      add: vi.fn(),
      edit: vi.fn(),
      remove: vi.fn(),
      refresh: vi.fn(),
      importConfig: vi.fn(),
      editConfig: vi.fn(),
      exportConfig: vi.fn(),
      mcp: vi.fn(),
      onboarding: vi.fn(),
    };

    renderSidebar({
      onAddStorage: handlers.add,
      onEditStorage: handlers.edit,
      onDeleteStorage: handlers.remove,
      onRefreshStorage: handlers.refresh,
      onImportStorages: handlers.importConfig,
      onEditStorageConfig: handlers.editConfig,
      onExportStorages: handlers.exportConfig,
      onOpenMcpSettings: handlers.mcp,
      onOpenOnboarding: handlers.onboarding,
    });

    for (const [label, handler] of [
      ["Add Storage", handlers.add],
      ["Import Config", handlers.importConfig],
      ["Edit Config JSON", handlers.editConfig],
      ["Download Config", handlers.exportConfig],
      ["MCP Settings", handlers.mcp],
      ["Setup Guide", handlers.onboarding],
    ] as const) {
      fireEvent.pointerDown(screen.getByRole("button", { name: "Storage actions" }));
      fireEvent.click(await screen.findByText(label));
      expect(handler).toHaveBeenCalled();
    }

    fireEvent.contextMenu(screen.getByRole("button", { name: /Google Bucket/i }));
    fireEvent.click(await screen.findByText("Refresh"));
    expect(handlers.refresh).toHaveBeenCalledWith("gcs");

    fireEvent.contextMenu(screen.getByRole("button", { name: /Google Bucket/i }));
    fireEvent.click(await screen.findByText("Edit"));
    expect(handlers.edit).toHaveBeenCalledWith("gcs");

    fireEvent.contextMenu(screen.getByRole("button", { name: /Google Bucket/i }));
    fireEvent.click(await screen.findByText("Delete"));
    expect(handlers.remove).toHaveBeenCalledWith("gcs");
  });

  it("copies internally dropped files between storages", async () => {
    vi.mocked(transferEntries).mockResolvedValue(undefined);
    const dataTransfer = internalTransferDataTransfer({
      kind: "infimount-transfer",
      fromSourceId: "local",
      paths: ["/report.txt"],
      operation: "copy",
    });

    renderSidebar();

    const target = screen.getByRole("button", { name: /Google Bucket/i });
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });

    await waitFor(() => {
      expect(transferEntries).toHaveBeenCalledWith("local", "gcs", ["/report.txt"], "/", "copy", "fail");
      expect(toast).toHaveBeenCalledWith({
        title: "Copied",
        description: "1 item copied.",
      });
    });
  });

  it("offers conflict resolution for dropped files", async () => {
    vi.mocked(transferEntries)
      .mockRejectedValueOnce(new TauriApiError("Already exists", "ALREADY_EXISTS"))
      .mockResolvedValueOnce(undefined);
    const dataTransfer = internalTransferDataTransfer({
      kind: "infimount-transfer",
      fromSourceId: "local",
      paths: ["/report.txt"],
      operation: "move",
    });

    renderSidebar();

    fireEvent.drop(screen.getByRole("button", { name: /Google Bucket/i }), { dataTransfer });

    expect(await screen.findByText("Item already exists")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Overwrite" }));

    await waitFor(() => {
      expect(transferEntries).toHaveBeenLastCalledWith("local", "gcs", ["/report.txt"], "/", "move", "overwrite");
    });
  });

  it("opens and installs a pending app update", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    vi.mocked(check).mockResolvedValue({
      version: "0.3.0",
      currentVersion: "0.2.2",
      downloadAndInstall,
    } as unknown as Awaited<ReturnType<typeof check>>);

    renderSidebar();

    fireEvent.click(screen.getByTitle("Check for updates"));

    expect(await screen.findByText("Install update?")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Download & Install" })).not.toBeDisabled();
    });
    fireEvent.click(screen.getByRole("button", { name: "Download & Install" }));

    await waitFor(() => {
      expect(downloadAndInstall).toHaveBeenCalled();
      expect(toast).toHaveBeenCalledWith({
        title: "Update installed",
        description: "Restart Infimount to apply the new version.",
      });
    });
  });
});
