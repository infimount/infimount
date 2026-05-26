import { expect, test } from "@playwright/experimental-ct-react";

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
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={async () => undefined}
        onStartHttp={async () => undefined}
        onStopHttp={async () => undefined}
        onTestServer={async () => undefined}
        onRefreshAudit={async () => undefined}
        onClearAudit={async () => undefined}
        onApproveConfirmation={async () => undefined}
        onDenyConfirmation={async () => undefined}
        onEnableNotifications={async () => undefined}
        onUpdateStoragePolicy={async () => undefined}
      />
    </div>,
  );

  await expect(page.getByText("MCP Settings")).toBeVisible();
  await expect(page).toHaveScreenshot("mcp-settings-dialog.png");
});
