import { expect, test } from "@playwright/experimental-ct-react";

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
  stdio: `{
  "mcpServers": {
    "infimount": {
      "command": "infimount_mcp",
      "args": ["--transport", "stdio"]
    }
  }
}`,
  http: `{
  "mcpServers": {
    "infimount": {
      "url": "http://127.0.0.1:7331/mcp"
    }
  }
}`,
};

const tools: McpToolDefinition[] = [
  { name: "list_dir", description: "List directories within the Infimount virtual filesystem." },
  { name: "export_config", description: "Export the storage registry as JSON." },
];

test("renders the MCP settings dialog", async ({ mount, page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "confirm", {
      configurable: true,
      value: () => true,
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async () => undefined,
      },
    });
  });

  await mount(
    <div className="min-h-screen bg-background p-8">
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={status}
        snippets={snippets}
        tools={tools}
        onSave={async () => undefined}
        onStartHttp={async () => undefined}
        onStopHttp={async () => undefined}
      />
    </div>,
  );

  await expect(page.getByText("MCP Settings")).toBeVisible();
  await expect(page).toHaveScreenshot("mcp-settings-dialog.png");
});
