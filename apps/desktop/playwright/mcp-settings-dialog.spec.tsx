import { expect, test } from "@playwright/experimental-ct-react";

import { McpSettingsDialog } from "@/components/McpSettingsDialog";
import type {
  McpClientSnippets,
  McpRuntimeStatus,
  McpSettingsUpdate,
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
    enabledTools: ["list_dir"],
    maxReadBytes: 2097152,
    maxWriteBytes: 4194304,
    securityBaselineVersion: 2,
    authTokenConfigured: false,
  },
  runningHttp: false,
  endpoint: null,
  endpointDisplay: "http://127.0.0.1:7331/mcp",
  authTokenConfigured: false,
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
  {
    name: "list_dir",
    description: "List directories within the Infimount virtual filesystem.",
    category: "read",
    risk: "low",
    defaultEnabled: true,
  },
  {
    name: "delete_path",
    description: "Delete a file or recursively delete a directory.",
    category: "destructive",
    risk: "high",
    defaultEnabled: false,
  },
];

const mcpPolicy: McpStoragePolicy = {
  version: 2,
  default_access: "read_write",
  rules: [],
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
        onExportAuditBundle={async () => undefined}
        onApproveConfirmation={async () => undefined}
        onDenyConfirmation={async () => undefined}
        onEnableNotifications={async () => undefined}
        onUpdateStoragePolicy={async () => undefined}
      />
    </div>,
  );

  await expect(page.getByText("MCP Settings")).toBeVisible();
  await expect(page.getByRole("button", { name: "Enable all" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Apply safe read-only" })).toBeVisible();
  await expect(page).toHaveScreenshot("mcp-settings-dialog.png");
  await page.getByRole("button", { name: "Configure advanced tools" }).click();
  await expect(page.getByText("delete_path")).toBeVisible();
  await page.getByRole("switch", { name: "Enable delete_path" }).click();
  await expect(page.getByText("Enable delete_path?")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("switch", { name: "Enable delete_path" })).not.toBeChecked();

  await page.getByRole("button", { name: /Manual Approval/i }).click();
  await expect(page.getByText("Apply Manual Approval?")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
});

test("saves lower file byte limits without enabling Agent Access", async ({ mount, page }, testInfo) => {
  let saved: McpSettingsUpdate | undefined;
  await mount(
    <div className="min-h-screen bg-background p-8">
      <McpSettingsDialog
        open
        onOpenChange={() => undefined}
        status={{ ...status, settings: { ...status.settings, transport: "stdio" } }}
        snippets={snippets}
        tools={tools}
        storages={storages}
        auditEvents={[]}
        pendingConfirmations={[]}
        activeSessions={[]}
        notificationPermission="default"
        onSave={async (update) => { saved = update; }}
        onStartHttp={async () => undefined}
        onStopHttp={async () => undefined}
        onTestServer={async () => undefined}
        onRefreshAudit={async () => undefined}
        onClearAudit={async () => undefined}
        onExportAuditBundle={async () => undefined}
        onApproveConfirmation={async () => undefined}
        onDenyConfirmation={async () => undefined}
        onEnableNotifications={async () => undefined}
        onUpdateStoragePolicy={async () => undefined}
      />
    </div>,
  );
  await page.getByLabel("Maximum read bytes").fill("131072");
  await page.getByLabel("Maximum write bytes").fill("262144");
  await page.getByRole("button", { name: "Save disabled state" }).click();
  await expect.poll(() => saved).toEqual({
    enabled: false,
    transport: "stdio",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: ["list_dir"],
    maxReadBytes: 131072,
    maxWriteBytes: 262144,
    authTokenMutation: { action: "keep" },
  });
  await expect(page.getByRole("button", { name: "Save disabled state" })).toBeEnabled();
  await expect(page.getByRole("switch", { name: "Enable general Agent Access" })).not.toBeChecked();
  await expect(page.getByLabel("Maximum read bytes")).toHaveValue("131072");
  await expect(page.getByLabel("Maximum write bytes")).toHaveValue("262144");
  const screenshot = testInfo.outputPath("__snapshots__/mcp-byte-limits-saved.png");
  await page.screenshot({ path: screenshot, animations: "disabled" });
  await testInfo.attach("mcp-byte-limits-saved", { path: screenshot, contentType: "image/png" });
  await expect(page).toHaveScreenshot("mcp-byte-limits-saved.png");
});
