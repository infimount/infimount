import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ActivationWizard } from "./ActivationWizard";
import {
  applyMcpClientInstall,
  listMcpClientAdapters,
  runActivationProbe,
  previewMcpClientInstall,
  rollbackMcpClientInstall,
  type WorkspaceRecord,
} from "@/lib/api";
import type { McpClientAdapterInfo, McpClientKind } from "@/types/storage";

vi.mock("@/lib/api", () => ({
  runActivationProbe: vi.fn(),
  listMcpClientAdapters: vi.fn(),
  previewMcpClientInstall: vi.fn(),
  applyMcpClientInstall: vi.fn(),
  rollbackMcpClientInstall: vi.fn(),
}));

const kinds: McpClientKind[] = [
  "generic_stdio",
  "claude_code",
  "cursor",
  "vs_code",
  "open_code",
  "claude_desktop",
];

const adapters: McpClientAdapterInfo[] = kinds.map((kind) => ({
  kind,
  name: {
    generic_stdio: "Generic stdio JSON",
    claude_code: "Claude Code",
    cursor: "Cursor",
    vs_code: "VS Code",
    open_code: "OpenCode",
    claude_desktop: "Claude Desktop",
  }[kind],
  description: `${kind} adapter`,
  detected: true,
  detection: "/verified/mcp",
  writeCapable: ["claude_code", "cursor", "vs_code", "open_code"].includes(kind),
  requiresExecutionConfirmation: kind === "claude_code" || kind === "vs_code",
  defaultTarget: kind === "cursor" ? "/tmp/.cursor/mcp.json" : null,
  snippet: '{"command":"/verified/mcp","args":["serve","--transport","stdio"]}',
}));

const workspace: WorkspaceRecord = {
  id: "workspace-id",
  schemaVersion: 2,
  storageId: "storage-id",
  name: "Agent workspace",
  rootPath: "/agent-workspaces/agent-workspace",
  templateId: "custom",
  accessProfile: "read_write",
  policyRuleId: "workspace:workspace-id",
  createdAt: "2026-09-11T00:00:00Z",
  updatedAt: "2026-09-11T00:00:00Z",
  memoryFiles: [],
  checkpointIds: [],
};

const successfulProbe = {
  sidecar: {
    binaryFound: true,
    executable: true,
    canonicalPath: "/mcp",
    version: "0.8.1-rc.5",
    versionMatch: true,
    doctorHealthy: true,
    sha256: "hash",
    checksumVerified: true,
    errorCode: null,
  },
  mcpHandshakeOk: true,
  mcpAllowedOpOk: true,
  mcpDenialProven: true,
  mcpAuditOk: true,
  scopeIsolationPassed: true,
  safeDefaultProfilePassed: true,
  advancedToolsEnabled: false,
  overallOk: true,
  errorCode: null,
} as const;

function renderWizard(overrides: Partial<React.ComponentProps<typeof ActivationWizard>> = {}) {
  return render(
    <ActivationWizard
      open
      onOpenChange={vi.fn()}
      onAddStorage={vi.fn()}
      onCreateDemo={vi.fn(async () => undefined)}
      onOpenWorkspaces={vi.fn()}
      onOpenMcpSettings={vi.fn()}
      onPrepareAgentAccess={vi.fn(async () => undefined)}
      onComplete={vi.fn(async () => undefined)}
      onSkip={vi.fn(async () => undefined)}
      onSaveState={vi.fn(async () => undefined)}
      storagesCount={1}
      workspaces={[workspace]}
      agentAccessReady={false}
      initialStep="client"
      initialCompletedSteps={["welcome", "storage", "workspace", "mcp"]}
      {...overrides}
    />,
  );
}

describe("ActivationWizard guided setup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listMcpClientAdapters).mockResolvedValue(adapters);
    vi.mocked(previewMcpClientInstall).mockResolvedValue({
      previewId: "preview-1",
      kind: "cursor",
      action: "write",
      targetPath: "/tmp/.cursor/mcp.json",
      before: '{"mcpServers":{"other":{}}}',
      after: '{"mcpServers":{"other":{},"infimount":{}}}',
      canApply: true,
      requiresExecutionConfirmation: false,
      expiresInSeconds: 600,
    });
    vi.mocked(applyMcpClientInstall).mockResolvedValue({
      applied: true,
      targetPath: "/tmp/.cursor/mcp.json",
      backupPath: "/tmp/.cursor/mcp.json.backup",
      rollbackId: "rollback-1",
    });
    vi.mocked(rollbackMcpClientInstall).mockResolvedValue(undefined);
    vi.mocked(runActivationProbe).mockResolvedValue(successfulProbe);
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it("mounts six distinct verified adapter cards inside a viewport-scrollable wizard", async () => {
    renderWizard();
    for (const adapter of adapters) {
      expect(await screen.findByTestId(`client-adapter-${adapter.kind}`)).toBeInTheDocument();
    }
    expect(screen.getAllByText(/serve/)).toHaveLength(6);
    expect(screen.getByRole("dialog")).toHaveClass("max-h-[min(90vh,760px)]", "overflow-hidden");
    expect(screen.getByTestId("wizard-scroll-region")).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-y-auto",
    );
  });

  it("previews, applies, and rolls back a writable adapter", async () => {
    renderWizard();
    const card = await screen.findByTestId("client-adapter-cursor");
    fireEvent.click(within(card).getByRole("button", { name: "Preview install" }));
    expect(await within(card).findByText("Reviewed write preview (secrets redacted)")).toBeInTheDocument();
    expect(previewMcpClientInstall).toHaveBeenCalledWith("cursor", "/tmp/.cursor/mcp.json");

    fireEvent.click(within(card).getByRole("button", { name: "Apply exact change" }));
    await waitFor(() => expect(applyMcpClientInstall).toHaveBeenCalledWith("preview-1", false));
    fireEvent.click(await within(card).findByRole("button", { name: "Roll back" }));
    await waitFor(() => expect(rollbackMcpClientInstall).toHaveBeenCalledWith("rollback-1"));
  });

  it("renders the completion state without enabling an unverified finish", () => {
    renderWizard({
      initialStep: "done",
      initialCompletedSteps: ["welcome", "storage", "workspace", "mcp", "client", "verify"],
    });
    expect(screen.getByText("Setup complete!")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  });

  it("renders the storage-first welcome and navigates safely", async () => {
    const onSaveState = vi.fn(async () => undefined);
    const onSkip = vi.fn(async () => undefined);
    renderWizard({ initialStep: "welcome", initialCompletedSteps: [], onSaveState, onSkip });
    expect(screen.getByText("Welcome to Infimount")).toBeInTheDocument();
    expect(screen.getByText("Browse storage")).toBeInTheDocument();
    expect(screen.queryByText(/Adding one does not expose it to an agent/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Continue/ }));
    await waitFor(() => expect(onSaveState).toHaveBeenCalledWith("storage", ["welcome"]));
    expect(screen.getByText(/Adding one does not expose it to an agent/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    await waitFor(() => expect(onSkip).toHaveBeenCalled());
  });

  it("handles empty storage setup without jumping directly to workspace setup", async () => {
    const onAddStorage = vi.fn();
    const onCreateDemo = vi.fn().mockRejectedValue(new Error("demo unavailable"));
    renderWizard({
      initialStep: "storage",
      initialCompletedSteps: ["welcome"],
      storagesCount: 0,
      workspaces: [],
      onAddStorage,
      onCreateDemo,
    });
    fireEvent.click(screen.getByRole("button", { name: "Add and validate storage" }));
    expect(onAddStorage).toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /Continue/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Create safe demo" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Demo setup failed");
  });

  it("prepares scoped agent access inline and keeps advanced MCP settings optional", async () => {
    const onPrepareAgentAccess = vi.fn(async () => undefined);
    const onOpenMcpSettings = vi.fn();
    renderWizard({
      initialStep: "mcp",
      initialCompletedSteps: ["welcome", "storage", "workspace"],
      onPrepareAgentAccess,
      onOpenMcpSettings,
    });

    expect(screen.getByRole("heading", { name: "Prepare agent access" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Advanced MCP settings" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Prepare agent access" }));
    await waitFor(() => expect(onPrepareAgentAccess).toHaveBeenCalledWith(workspace));
    await waitFor(() => expect(runActivationProbe).toHaveBeenCalled());
    expect(await screen.findByText(/passed version and doctor checks/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Continue/ })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "Advanced MCP settings" }));
    expect(onOpenMcpSettings).toHaveBeenCalled();
  });

  it("never reuses one workspace readiness for a different selected workspace", async () => {
    const secondWorkspace: WorkspaceRecord = {
      ...workspace,
      id: "workspace-two",
      storageId: "storage-two",
      name: "Second workspace",
      rootPath: "/agent-workspaces/second-workspace",
      accessProfile: "read_only",
      policyRuleId: "workspace:workspace-two",
    };
    const onPrepareAgentAccess = vi.fn(async () => undefined);

    renderWizard({
      initialStep: "mcp",
      initialCompletedSteps: ["welcome", "storage", "workspace"],
      workspaces: [workspace, secondWorkspace],
      agentAccessReady: true,
      onPrepareAgentAccess,
    });

    expect(screen.queryByText("Scoped agent access is ready.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Prepare agent access" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Continue/ })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Prepare agent access" }));
    await waitFor(() => expect(onPrepareAgentAccess).toHaveBeenCalledWith(workspace));
    expect(await screen.findByText("Scoped agent access is ready.")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Workspace for agent access"), {
      target: { value: secondWorkspace.id },
    });
    expect(screen.queryByText("Scoped agent access is ready.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Prepare agent access" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Continue/ })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Prepare agent access" }));
    await waitFor(() => expect(onPrepareAgentAccess).toHaveBeenCalledWith(secondWorkspace));
    expect(await screen.findByText("Scoped agent access is ready.")).toBeInTheDocument();
  });

  it("runs the safety probe and renders every verification check", async () => {
    renderWizard({
      initialStep: "verify",
      initialCompletedSteps: ["welcome", "storage", "workspace", "mcp", "client"],
    });
    expect(screen.getByText(/Run the safety probe/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Run safety probe" }));
    await screen.findByText("The packaged MCP sidecar passed the workspace access and policy-denial checks.");
    expect(screen.getAllByText(/verified|completed|allowed|denied|audited/i)).toHaveLength(5);
    fireEvent.click(screen.getByRole("button", { name: /Continue/ }));
    expect(await screen.findByText("Setup complete!")).toBeInTheDocument();

    const onComplete = vi.fn(async () => undefined);
    cleanup();
    renderWizard({
      initialStep: "verify",
      initialCompletedSteps: ["welcome", "storage", "workspace", "mcp", "client"],
      onComplete,
    });
    fireEvent.click(screen.getByRole("button", { name: "Run safety probe" }));
    await screen.findByText("The packaged MCP sidecar passed the workspace access and policy-denial checks.");
    fireEvent.click(screen.getByRole("button", { name: /Continue/ }));
    fireEvent.click(screen.getByRole("button", { name: "Finish" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalled());
  });

  it("marks copied snippets reviewed and reports preview and apply failures", async () => {
    renderWizard();
    const generic = await screen.findByTestId("client-adapter-generic_stdio");
    fireEvent.click(within(generic).getByRole("button", { name: "Copy" }));
    expect(await within(generic).findByRole("button", { name: "Copied!" })).toBeInTheDocument();
    expect(navigator.clipboard.writeText).toHaveBeenCalled();

    const cursor = await screen.findByTestId("client-adapter-cursor");
    vi.mocked(previewMcpClientInstall).mockRejectedValueOnce(new Error("preview unavailable"));
    fireEvent.click(within(cursor).getByRole("button", { name: "Preview install" }));
    expect(await within(cursor).findByText("preview unavailable")).toBeInTheDocument();

    vi.mocked(previewMcpClientInstall).mockResolvedValueOnce({
      previewId: "preview-confirm",
      kind: "cursor",
      action: "write",
      targetPath: "/tmp/.cursor/mcp.json",
      before: "",
      after: "{}",
      canApply: true,
      requiresExecutionConfirmation: true,
      expiresInSeconds: 60,
    });
    fireEvent.click(within(cursor).getByRole("button", { name: "Preview install" }));
    expect(await within(cursor).findByText(/I confirm execution/)).toBeInTheDocument();
    const apply = within(cursor).getByRole("button", { name: "Confirm and execute" });
    expect(apply).toBeDisabled();
    fireEvent.click(within(cursor).getByLabelText("I confirm execution of this exact command"));
    vi.mocked(applyMcpClientInstall).mockRejectedValueOnce(new Error("apply unavailable"));
    fireEvent.click(apply);
    expect(await within(cursor).findByText("apply unavailable")).toBeInTheDocument();
  });
});
