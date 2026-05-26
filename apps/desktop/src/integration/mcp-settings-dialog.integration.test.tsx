import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { McpSettingsDialog } from "@/components/McpSettingsDialog";
import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpStoragePolicy,
  McpToolDefinition,
  StorageConfig,
} from "@/types/storage";

const status: McpRuntimeStatus = {
  settings: {
    enabled: false,
    transport: "http",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: ["list_dir", "export_config"],
  },
  runningHttp: false,
  endpoint: null,
  endpointDisplay: "http://127.0.0.1:7331/mcp",
};

const snippets: McpClientSnippets = {
  stdio: '{ "mcpServers": { "infimount": { "command": "infimount_mcp" } } }',
  http: '{ "mcpServers": { "infimount": { "url": "http://127.0.0.1:7331/mcp" } } }',
};

const tools: McpToolDefinition[] = [
  { name: "list_dir", description: "List directories within the Infimount virtual filesystem." },
  { name: "stat_path", description: "Return path metadata." },
  { name: "read_file", description: "Read file contents." },
  { name: "search_paths", description: "Search storage paths." },
  { name: "write_file", description: "Write file contents." },
  { name: "mkdir", description: "Create directories." },
  { name: "copy_path", description: "Copy files." },
  { name: "move_path", description: "Move files." },
  { name: "delete_path", description: "Delete files." },
  { name: "generate_download_link", description: "Create download links." },
  { name: "list_storages", description: "List exposed storage." },
  { name: "validate_storage", description: "Validate storage configuration." },
  { name: "export_config", description: "Export the storage registry as JSON." },
];

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
    name: "Local",
    backend: "local",
    type: "local-fs",
    config: {},
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    connected: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    mcpPolicy,
  },
];

describe("McpSettingsDialog integration", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
  });

  it("saves settings before starting the HTTP server", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onStartHttp = vi.fn().mockResolvedValue(undefined);

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={onSave}
        onStartHttp={onStartHttp}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Save & Start HTTP Server/i }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith({
        enabled: true,
        transport: "http",
        bindAddress: "127.0.0.1",
        port: 7331,
        enabledTools: ["list_dir", "export_config"],
      });
      expect(onStartHttp).toHaveBeenCalled();
    });

    expect(onSave.mock.invocationCallOrder[0]).toBeLessThan(
      onStartHttp.mock.invocationCallOrder[0],
    );
  });

  it("shows a non-loopback warning before starting HTTP", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onStartHttp = vi.fn().mockResolvedValue(undefined);

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={onSave}
        onStartHttp={onStartHttp}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByDisplayValue("127.0.0.1"), {
      target: { value: "0.0.0.0" },
    });
    expect(screen.getByText(/not loopback/i)).toBeInTheDocument();
    expect(screen.getByText("Review network exposure")).toBeInTheDocument();
    expect(screen.getByText("Non-loopback bind needs review")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Save & Start HTTP Server/i }));

    expect(await screen.findByText(/Expose MCP beyond this machine/i)).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
    expect(onStartHttp).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /^Start Server$/i }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith({
        enabled: true,
        transport: "http",
        bindAddress: "0.0.0.0",
        port: 7331,
        enabledTools: ["list_dir", "export_config"],
      });
      expect(onStartHttp).toHaveBeenCalled();
    });
  });

  it("renders pending MCP approvals and routes approve or deny actions", async () => {
    const onApproveConfirmation = vi.fn().mockResolvedValue(undefined);
    const onDenyConfirmation = vi.fn().mockResolvedValue(undefined);

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[
          {
            operation_id: "op-1",
            tool_name: "delete_path",
            operation: "delete",
            risk_type: "delete",
            storage_id: "local",
            storage_name: "Local",
            path: "/Local/file.txt",
            summary: "delete_path on /Local/file.txt",
            created_at: "2026-01-01T00:00:00Z",
            expires_at: "2026-01-01T00:05:00Z",
          },
        ]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={vi.fn()}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={onApproveConfirmation}
        onDenyConfirmation={onDenyConfirmation}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    expect(screen.getByText(/Pending MCP Approvals/i)).toBeInTheDocument();
    expect(screen.getAllByText("delete_path").length).toBeGreaterThan(0);
    expect(screen.getByText(/delete_path on \/Local\/file.txt/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Approve/i }));
    fireEvent.click(screen.getByRole("button", { name: /Deny/i }));

    await waitFor(() => {
      expect(onApproveConfirmation).toHaveBeenCalledWith("op-1");
      expect(onDenyConfirmation).toHaveBeenCalledWith("op-1");
    });
  });

  it("renders active scoped MCP sessions", () => {
    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[
          {
            id: "session-1234567890",
            allowed_storages: ["Local"],
            allowed_prefixes: ["docs"],
            read_only: true,
            created_at: "2026-01-01T00:00:00Z",
            expires_at: "2026-01-01T01:00:00Z",
          },
        ]}
        notificationPermission="default"
        onSave={vi.fn()}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    expect(screen.getByText("Active MCP Sessions")).toBeInTheDocument();
    expect(screen.getAllByText("1").length).toBeGreaterThan(0);
    expect(screen.getByText(/Local,\s+prefixes: docs/i)).toBeInTheDocument();
    expect(screen.getAllByText("read-only").length).toBeGreaterThan(0);
  });

  it("makes the agent access summary policy-aware", () => {
    const writeEnabledStatus: McpRuntimeStatus = {
      ...status,
      settings: {
        ...status.settings,
        enabledTools: ["list_dir", "write_file", "delete_path", "generate_download_link"],
      },
    };
    const readOnlyStorage: StorageConfig = {
      ...storages[0],
      readOnly: true,
      mcpPolicy: { ...mcpPolicy, default_access: "read_write" },
    };
    const noAccessStorage: StorageConfig = {
      ...storages[0],
      id: "blocked",
      name: "Blocked",
      readOnly: false,
      mcpPolicy: { ...mcpPolicy, default_access: "none" },
    };

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={writeEnabledStatus}
        snippets={snippets}
        tools={tools}
        storages={[readOnlyStorage, noAccessStorage]}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={vi.fn()}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    expect(screen.getByText("Ready: scoped local access")).toBeInTheDocument();
    expect(screen.getByText("0 write / 1 read-only / 1 no access")).toBeInTheDocument();
    expect(screen.getAllByText("read-only").length).toBeGreaterThan(0);
    expect(screen.getAllByText("no access").length).toBeGreaterThan(0);
    expect(screen.getAllByText("4 approval rules").length).toBeGreaterThan(0);
    expect(screen.getByText("Allowed for 1 storage")).toBeInTheDocument();
    expect(screen.getAllByText("Blocked by read-only or no-access policies")).toHaveLength(2);
    expect(screen.getByText("Enabled for 1 storage")).toBeInTheDocument();
    expect(screen.getByText(/writes, deletes, links, cross-storage copy require approval/i)).toBeInTheDocument();
  });

  it("applies safe access presets to tools and exposed storage policies", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onUpdateStoragePolicy = vi.fn().mockResolvedValue(undefined);

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={onSave}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={onUpdateStoragePolicy}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Read-only research/i }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith(
        expect.objectContaining({
          enabled: false,
          enabledTools: expect.arrayContaining(["list_dir", "read_file", "search_paths"]),
        }),
      );
      expect(onUpdateStoragePolicy).toHaveBeenCalledWith("local", {
        ...mcpPolicy,
        default_access: "read_only",
      });
    });

    expect(onSave.mock.calls[0][0].enabledTools).not.toContain("write_file");
  });

  it("filters, copies, and exports visible MCP audit events", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const onExportAuditBundle = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: { writeText },
    });

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[
          {
            id: "audit-1",
            timestamp: "2026-01-01T00:00:00Z",
            actor_type: "mcp_client",
            mcp_client_id: null,
            session_id: "session-1",
            storage_id: "local",
            storage_name: "Local",
            backend: "local",
            tool_name: "read_file",
            operation: "read",
            path: "/Local/docs/readme.md",
            version_id: null,
            decision: "allowed",
            confirmation_id: null,
            duration_ms: 1,
            bytes_read: 12,
            bytes_written: null,
            error_code: null,
          },
          {
            id: "audit-2",
            timestamp: "2026-01-01T00:01:00Z",
            actor_type: "mcp_client",
            mcp_client_id: null,
            session_id: null,
            storage_id: "local",
            storage_name: "Local",
            backend: "local",
            tool_name: "delete_path",
            operation: "delete",
            path: "/Local/private.txt",
            version_id: null,
            decision: "denied",
            confirmation_id: null,
            duration_ms: 2,
            bytes_read: null,
            bytes_written: null,
            error_code: "ERR_MCP_POLICY_DENIED",
          },
        ]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={vi.fn()}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={onExportAuditBundle}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText(/Filter tool, path, error, or session/i), {
      target: { value: "private" },
    });

    expect(screen.getByText(/Showing 1 of 1 matching events/i)).toBeInTheDocument();
    expect(screen.getAllByText("delete_path").length).toBeGreaterThan(0);
    expect(screen.queryByText(/docs\/readme/i)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Copy visible/i }));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining("audit-2"));
    });
    expect(writeText.mock.calls[0][0]).not.toContain("audit-1");

    fireEvent.click(screen.getByRole("button", { name: /Export visible/i }));

    await waitFor(() => {
      expect(onExportAuditBundle).toHaveBeenCalledWith([
        expect.objectContaining({ id: "audit-2" }),
      ]);
    });
  });

  it("edits and saves path policy for an exposed storage", async () => {
    const onUpdateStoragePolicy = vi.fn().mockResolvedValue(undefined);

    render(
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={vi.fn()}
        onStartHttp={vi.fn()}
        onStopHttp={vi.fn()}
        onTestServer={vi.fn()}
        onRefreshAudit={vi.fn()}
        onClearAudit={vi.fn()}
        onExportAuditBundle={vi.fn()}
        onApproveConfirmation={vi.fn()}
        onDenyConfirmation={vi.fn()}
        onEnableNotifications={vi.fn()}
        onUpdateStoragePolicy={onUpdateStoragePolicy}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText(/Leave empty to allow all paths/i), {
      target: { value: "docs\n./shared/\nshared" },
    });
    fireEvent.change(screen.getByPlaceholderText(/Example: private/i), {
      target: { value: "/private\nsecrets\nprivate" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Save policy/i }));

    await waitFor(() => {
      expect(onUpdateStoragePolicy).toHaveBeenCalledWith("local", {
        ...mcpPolicy,
        allowed_paths: ["docs", "shared"],
        denied_paths: ["private", "secrets"],
      });
    });
  });
});
