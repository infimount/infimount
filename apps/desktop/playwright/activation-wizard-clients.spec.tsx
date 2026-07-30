import { expect, test } from "@playwright/experimental-ct-react";
import { ActivationWizard } from "@/components/ActivationWizard";

const kinds = [
  ["generic_stdio", "Generic stdio JSON", false],
  ["claude_code", "Claude Code", true],
  ["cursor", "Cursor", true],
  ["vs_code", "VS Code", true],
  ["open_code", "OpenCode", true],
  ["claude_desktop", "Claude Desktop", false],
] as const;

test("mounts all client adapters and applies a reviewed Cursor merge", async ({ mount, page }) => {
  const installMocks = (adapterKinds: typeof kinds) => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {
        invoke: async (cmd: string) => {
          if (cmd === "list_mcp_client_adapters") {
            return adapterKinds.map(([kind, name, writeCapable]) => ({
              kind,
              name,
              description: `${name} adapter`,
              detected: true,
              detection: "/verified/bundled/mcp",
              writeCapable,
              requiresExecutionConfirmation: kind === "claude_code" || kind === "vs_code",
              defaultTarget: kind === "cursor" ? "/project/.cursor/mcp.json" : null,
              snippet: '{"command":"/verified/bundled/mcp","args":["serve","--transport","stdio"]}',
            }));
          }
          if (cmd === "preview_mcp_client_install") {
            return {
              previewId: "preview-1",
              kind: "cursor",
              action: "write",
              targetPath: "/project/.cursor/mcp.json",
              before: '{"mcpServers":{"other":{}}}',
              after: '{"mcpServers":{"other":{},"infimount":{}}}',
              canApply: true,
              requiresExecutionConfirmation: false,
              expiresInSeconds: 600,
            };
          }
          if (cmd === "apply_mcp_client_install") {
            return {
              applied: true,
              targetPath: "/project/.cursor/mcp.json",
              backupPath: "/project/.cursor/mcp.json.backup",
              rollbackId: "rollback-1",
            };
          }
          if (cmd === "rollback_mcp_client_install") return null;
          return null;
        },
        transformCallback: (() => {
          let nextId = 1;
          return () => nextId++;
        })(),
        unregisterCallback: () => undefined,
      },
    });
  };
  await page.addInitScript(installMocks, kinds);
  await page.evaluate(installMocks, kinds);

  await mount(
    <ActivationWizard
      open
      onOpenChange={() => undefined}
      onAddStorage={() => undefined}
      onCreateDemo={async () => undefined}
      onOpenWorkspaces={() => undefined}
      onOpenMcpSettings={() => undefined}
      onComplete={async () => undefined}
      onSkip={async () => undefined}
      onSaveState={async () => undefined}
      storagesCount={1}
      workspacesCount={1}
      initialStep="client"
      initialCompletedSteps={["welcome", "storage", "workspace", "mcp"]}
    />,
  );

  for (const [kind, name] of kinds) {
    await expect(page.getByTestId(`client-adapter-${kind}`)).toContainText(name);
  }

  const cursor = page.getByTestId("client-adapter-cursor");
  await cursor.getByRole("button", { name: "Preview install" }).click();
  await expect(cursor).toContainText("Reviewed write preview (secrets redacted)");
  await expect(cursor).toContainText('"other"');
  await cursor.getByRole("button", { name: "Apply exact change" }).click();
  await cursor.getByRole("button", { name: "Roll back" }).click();
  await expect(cursor.getByRole("button", { name: "Roll back" })).toHaveCount(0);
});
