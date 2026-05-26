import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import Index from "@/pages/Index";
import type { McpRuntimeStatus, StorageDraft } from "@/types/storage";
import {
  addStorage,
  getAppSettings,
  getMcpClientSnippets,
  getMcpStatus,
  listMcpAuditEvents,
  listMcpTools,
  listPendingMcpConfirmations,
  listStorageSchemas,
  listStorages,
  startMcpHttp,
  updateMcpSettings,
} from "@/lib/api";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("0.3.0"),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/components/WindowControls", () => ({
  WindowControls: () => null,
}));

vi.mock("@/components/FileBrowser", () => ({
  FileBrowser: ({ sourceId, storageName }: { sourceId: string; storageName: string }) => (
    <section aria-label="release file browser smoke">
      Browsing {storageName} ({sourceId})
    </section>
  ),
}));

vi.mock("@/components/StorageSidebar", () => ({
  StorageSidebar: ({
    storages,
    selectedStorage,
    onSelectStorage,
    onAddStorage,
    onOpenMcpSettings,
  }: {
    storages: Array<{ id: string; name: string }>;
    selectedStorage: string | null;
    onSelectStorage: (id: string) => void;
    onAddStorage: () => void;
    onOpenMcpSettings?: () => void;
  }) => (
    <aside aria-label="release storage sidebar smoke">
      <button type="button" onClick={onAddStorage}>
        Open Add Storage
      </button>
      <button type="button" disabled={storages.length === 0} onClick={onOpenMcpSettings}>
        Open MCP Settings
      </button>
      {storages.map((storage) => (
        <button
          key={storage.id}
          type="button"
          aria-pressed={selectedStorage === storage.id}
          onClick={() => onSelectStorage(storage.id)}
        >
          {storage.name}
        </button>
      ))}
    </aside>
  ),
}));

vi.mock("@/lib/api", () => ({
  addStorage: vi.fn(),
  approveMcpConfirmation: vi.fn(),
  clearMcpAuditEvents: vi.fn(),
  completeOnboarding: vi.fn(),
  denyMcpConfirmation: vi.fn(),
  exportStorageConfig: vi.fn(),
  getAppSettings: vi.fn(),
  getMcpClientSnippets: vi.fn(),
  getMcpStatus: vi.fn(),
  importStorageConfig: vi.fn(),
  listEntries: vi.fn(),
  listMcpAuditEvents: vi.fn(),
  listMcpTools: vi.fn(),
  listPendingMcpConfirmations: vi.fn(),
  listStorageSchemas: vi.fn(),
  listStorages: vi.fn(),
  removeStorage: vi.fn(),
  skipOnboarding: vi.fn(),
  startMcpHttp: vi.fn(),
  stopMcpHttp: vi.fn(),
  transferEntries: vi.fn(),
  updateMcpSettings: vi.fn(),
  updateMcpStoragePolicy: vi.fn(),
  updateStorage: vi.fn(),
  verifyStorage: vi.fn(),
  TauriApiError: class extends Error {
    code: string;
    constructor(message: string, code = "UNKNOWN") {
      super(message);
      this.code = code;
    }
  },
}));

type StorageWire = {
  id: string;
  name: string;
  backend: StorageDraft["backend"];
  config: Record<string, unknown>;
  enabled: boolean;
  mcp_exposed: boolean;
  read_only: boolean;
  mcp_policy: Record<string, unknown>;
  created_at: string;
  updated_at: string;
};

const defaultPolicy = {
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

const mcpStatus: McpRuntimeStatus = {
  settings: {
    enabled: false,
    transport: "http",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: ["list_dir", "read_file"],
  },
  runningHttp: false,
  endpoint: null,
  endpointDisplay: "http://127.0.0.1:7331/mcp",
};

describe("release zero-manual smoke path", () => {
  let storages: StorageWire[];

  beforeEach(() => {
    vi.clearAllMocks();
    storages = [];
    window.localStorage.clear();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        addListener: () => undefined,
        removeListener: () => undefined,
        dispatchEvent: () => false,
      }),
    });

    vi.mocked(getAppSettings).mockResolvedValue({
      onboardingCompleted: true,
      onboardingSkipped: false,
      onboardingCompletedAt: "2026-05-24T00:00:00Z",
      onboardingSkippedAt: null,
    });
    vi.mocked(listStorageSchemas).mockResolvedValue([
      {
        id: "local-fs",
        label: "Local Filesystem",
        kind: "local",
        fields: [
          { name: "root", label: "Root path", input_type: "text", required: true },
        ],
      },
    ]);
    vi.mocked(listStorages).mockImplementation(
      async () => storages as unknown as Awaited<ReturnType<typeof listStorages>>,
    );
    vi.mocked(addStorage).mockImplementation(async (draft: StorageDraft) => {
      const added: StorageWire = {
        id: "local-1",
        name: draft.name,
        backend: draft.backend,
        config: draft.config,
        enabled: draft.enabled,
        mcp_exposed: draft.mcpExposed,
        read_only: draft.readOnly,
        mcp_policy: defaultPolicy,
        created_at: "2026-05-24T00:00:00Z",
        updated_at: "2026-05-24T00:00:00Z",
      };
      storages = [added];
      return added as unknown as Awaited<ReturnType<typeof addStorage>>;
    });
    vi.mocked(getMcpStatus).mockResolvedValue(mcpStatus);
    vi.mocked(getMcpClientSnippets).mockResolvedValue({
      stdio: '{"mcpServers":{"infimount":{"command":"infimount_mcp"}}}',
      http: '{"mcpServers":{"infimount":{"url":"http://127.0.0.1:7331/mcp"}}}',
    });
    vi.mocked(listMcpTools).mockResolvedValue([
      { name: "list_dir", description: "List directories" },
      { name: "read_file", description: "Read files" },
    ]);
    vi.mocked(listMcpAuditEvents).mockResolvedValue([]);
    vi.mocked(listPendingMcpConfirmations).mockResolvedValue([]);
    vi.mocked(updateMcpSettings).mockResolvedValue({
      ...mcpStatus,
      settings: { ...mcpStatus.settings, enabled: true },
    });
    vi.mocked(startMcpHttp).mockResolvedValue({
      ...mcpStatus,
      settings: { ...mcpStatus.settings, enabled: true },
      runningHttp: true,
      endpoint: "http://127.0.0.1:7331/mcp",
    });
  });

  it("adds a local storage, opens the browser, and starts MCP through the app shell", async () => {
    render(<Index />);

    expect(await screen.findByText("Select a storage to view files")).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "Open Add Storage" })[0]);

    fireEvent.change(await screen.findByLabelText("Storage Name"), {
      target: { value: "Release Local" },
    });
    fireEvent.change(screen.getByLabelText(/Root path/), {
      target: { value: "/tmp/infimount-release-smoke" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(addStorage).toHaveBeenCalledWith({
        name: "Release Local",
        backend: "local",
        config: { root: "/tmp/infimount-release-smoke" },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
      expect(listStorages).toHaveBeenCalledTimes(2);
    });

    expect(await screen.findByText("Browsing Release Local (local-1)")).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "Open MCP Settings" })[0]);

    expect(await screen.findByText("2 of 2 functions enabled")).toBeInTheDocument();
    expect(screen.getAllByText("Release Local").length).toBeGreaterThan(0);
    expect(screen.getAllByText("read/write").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: /Save & Start HTTP Server/i }));

    await waitFor(() => {
      expect(updateMcpSettings).toHaveBeenCalledWith({
        enabled: true,
        transport: "http",
        bindAddress: "127.0.0.1",
        port: 7331,
        enabledTools: ["list_dir", "read_file"],
      });
      expect(startMcpHttp).toHaveBeenCalled();
    });
  });
});
