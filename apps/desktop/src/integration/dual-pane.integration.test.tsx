import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import Index from "@/pages/Index";
import type { McpRuntimeStatus } from "@/types/storage";
import { getAppSettings, getMcpClientSnippets, getMcpStatus, listStorages } from "@/lib/api";

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
  FileBrowser: ({
    sourceId,
    storageName,
    onToggleDualPane,
    isDualPane,
  }: {
    sourceId: string;
    storageName: string;
    onToggleDualPane?: () => void;
    isDualPane?: boolean;
  }) => (
    <section aria-label={`browser ${storageName}`} data-source-id={sourceId}>
      Browsing {storageName}
      {onToggleDualPane ? (
        <button type="button" onClick={onToggleDualPane}>
          {isDualPane ? "Close split pane" : "Open split pane"}
        </button>
      ) : null}
    </section>
  ),
}));

vi.mock("@/components/StorageSidebar", () => ({
  StorageSidebar: ({
    storages,
    selectedStorage,
    onSelectStorage,
  }: {
    storages: Array<{ id: string; name: string }>;
    selectedStorage: string | null;
    onSelectStorage: (id: string) => void;
  }) => (
    <aside aria-label="storage sidebar">
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
  connectOAuthStorage: vi.fn(),
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
  listMcpAuditEvents: vi.fn().mockResolvedValue([]),
  listMcpTools: vi.fn().mockResolvedValue([]),
  listPendingMcpConfirmations: vi.fn().mockResolvedValue([]),
  listStorageSchemas: vi.fn().mockResolvedValue([]),
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

const mcpStatus: McpRuntimeStatus = {
  settings: {
    enabled: false,
    transport: "http",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: [],
    securityBaselineVersion: 2,
    authTokenConfigured: false,
  },
  runningHttp: false,
  endpoint: null,
  endpointDisplay: "http://127.0.0.1:7331/mcp",
  authTokenConfigured: false,
};

const policy = {
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

describe("dual-pane browsing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: () => ({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    });
    vi.mocked(getAppSettings).mockResolvedValue({
      onboardingCompleted: true,
      onboardingSkipped: false,
      onboardingCompletedAt: "2026-01-01T00:00:00Z",
      onboardingSkippedAt: null,
    });
    vi.mocked(getMcpStatus).mockResolvedValue(mcpStatus);
    vi.mocked(getMcpClientSnippets).mockResolvedValue({ stdio: "{}", http: "{}" });
    vi.mocked(listStorages).mockResolvedValue([
      {
        id: "local",
        name: "Local Docs",
        backend: "local",
        config: { root: "/tmp/docs" },
        enabled: true,
        mcp_exposed: true,
        read_only: false,
        mcp_policy: policy,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "archive",
        name: "Archive Bucket",
        backend: "s3",
        config: { bucket: "archive" },
        enabled: true,
        mcp_exposed: true,
        read_only: false,
        mcp_policy: policy,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
    ] as unknown as Awaited<ReturnType<typeof listStorages>>);
  });

  it("opens a same-storage side pane under one split header", async () => {
    render(<Index />);

    expect(await screen.findByLabelText("browser Local Docs")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open split pane" }));

    expect(await screen.findAllByText(/Browsing Local Docs/)).toHaveLength(2);
    expect(screen.getByText("Split view, two panes in the same storage")).toBeInTheDocument();
    expect(screen.queryByLabelText("Destination pane")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("browser Archive Bucket")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Close split pane" }));
    expect(await screen.findAllByText(/Browsing Local Docs/)).toHaveLength(1);
  });
});
