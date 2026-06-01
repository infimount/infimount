import { expect, test } from "@playwright/experimental-ct-react";

import { AppZoomProvider } from "@/hooks/use-app-zoom";
import { TransferQueueProvider } from "@/hooks/use-transfer-queue";
import { SplitPaneHarness } from "@/test/SplitPaneHarness";

const entriesByPath: Record<string, unknown[]> = {
  "/": [
    {
      path: "/docs",
      name: "docs",
      is_dir: true,
      size: 0,
      modified_at: "2026-01-01T00:00:00Z",
    },
    {
      path: "/readme.md",
      name: "readme.md",
      is_dir: false,
      size: 1200,
      modified_at: "2026-01-01T00:00:00Z",
    },
  ],
  "/docs": [
    {
      path: "/docs/guide.md",
      name: "guide.md",
      is_dir: false,
      size: 2048,
      modified_at: "2026-01-01T00:00:00Z",
    },
  ],
};

test("split pane opens as a visible same-storage native browsing mode", async ({ mount, page }) => {
  const installTauriMocks = ({ entriesByPath }: { entriesByPath: Record<string, unknown[]> }) => {
    Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
      configurable: true,
      value: {
        unregisterListener: () => undefined,
      },
    });
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { windowLabel: "main", label: "main" },
        },
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === "plugin:app|version") return "0.7.0";
          if (cmd === "plugin:event|listen") return args?.handler ?? 1;
          if (cmd === "plugin:event|unlisten") return null;
          if (cmd.includes("updater") || cmd.includes("window")) return null;
          if (cmd === "list_entries") {
            const path = typeof args?.path === "string" ? args.path : "/";
            return entriesByPath[path] ?? [];
          }
          if (cmd === "list_entries_recursive") return entriesByPath["/docs"] ?? [];
          return null;
        },
        transformCallback: (() => {
          let nextId = 1;
          return () => nextId++;
        })(),
        unregisterCallback: () => undefined,
      },
    });

    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: query.includes("max-width: 767px") ? false : false,
        media: query,
        onchange: null,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        addListener: () => undefined,
        removeListener: () => undefined,
        dispatchEvent: () => false,
      }),
    });
  };

  await page.addInitScript(installTauriMocks, { entriesByPath });
  await page.evaluate(installTauriMocks, { entriesByPath });

  await mount(
    <AppZoomProvider>
      <TransferQueueProvider>
        <div className="h-screen w-screen bg-background p-4">
          <SplitPaneHarness />
        </div>
      </TransferQueueProvider>
    </AppZoomProvider>,
  );

  await expect(page.getByLabel("Open split pane")).toBeVisible();
  await page.getByLabel("Open split pane").click();

  await expect(page.getByText("Split view, two panes in the same storage")).toBeVisible();
  await expect(page.getByRole("button", { name: "Close split pane" })).toBeVisible();
  await expect(page.getByText("Left")).toBeVisible();
  await expect(page.getByText("Right")).toBeVisible();
  await expect(page.getByLabel("Destination pane")).toHaveCount(0);
  await expect(page.getByText("Archive Bucket")).toHaveCount(0);
  await expect(page.getByText("Local Docs").first()).toBeVisible();

  await expect(page).toHaveScreenshot("split-pane-same-storage.png");

  await page.getByRole("button", { name: "Close split pane" }).click();
  await expect(page.getByText("Split view, two panes in the same storage")).toHaveCount(0);
});
