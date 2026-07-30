import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ActivationWizard } from "./ActivationWizard";
import {
  applyMcpClientInstall,
  listMcpClientAdapters,
  previewMcpClientInstall,
  rollbackMcpClientInstall,
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

function renderWizard() {
  return render(
    <ActivationWizard
      open
      onOpenChange={vi.fn()}
      onAddStorage={vi.fn()}
      onCreateDemo={vi.fn(async () => undefined)}
      onOpenWorkspaces={vi.fn()}
      onOpenMcpSettings={vi.fn()}
      onComplete={vi.fn(async () => undefined)}
      onSkip={vi.fn(async () => undefined)}
      onSaveState={vi.fn(async () => undefined)}
      storagesCount={1}
      workspacesCount={1}
      initialStep="client"
      initialCompletedSteps={["welcome", "storage", "workspace", "mcp"]}
    />,
  );
}

describe("ActivationWizard client adapters", () => {
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
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it("mounts six distinct verified adapter cards", async () => {
    renderWizard();
    for (const adapter of adapters) {
      expect(await screen.findByTestId(`client-adapter-${adapter.kind}`)).toBeInTheDocument();
    }
    expect(screen.getAllByText(/serve/)).toHaveLength(6);
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
});
