import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { McpSettingsDialog } from "@/components/McpSettingsDialog";
import type { McpClientSnippets, McpRuntimeStatus, McpToolDefinition } from "@/types/storage";

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
  { name: "export_config", description: "Export the storage registry as JSON." },
];

describe("McpSettingsDialog integration", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.stubGlobal("confirm", vi.fn(() => true));
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
        onSave={onSave}
        onStartHttp={onStartHttp}
        onStopHttp={vi.fn()}
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

    expect(onSave.mock.invocationCallOrder[0]).toBeLessThan(onStartHttp.mock.invocationCallOrder[0]);
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
        onSave={onSave}
        onStartHttp={onStartHttp}
        onStopHttp={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByDisplayValue("127.0.0.1"), {
      target: { value: "0.0.0.0" },
    });
    expect(screen.getByText(/not loopback/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Save & Start HTTP Server/i }));

    await waitFor(() => {
      expect(global.confirm).toHaveBeenCalled();
    });
  });
});
