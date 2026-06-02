import { expect, test } from "@playwright/experimental-ct-react";

import { DeleteProgressHarness } from "@/test/DeleteProgressHarness";

const entries = [
  {
    path: "/demo",
    name: "demo",
    is_dir: true,
    size: 0,
    modified_at: "2026-01-01T00:00:00Z",
  },
  {
    path: "/report.txt",
    name: "report.txt",
    is_dir: false,
    size: 1200,
    modified_at: "2026-01-01T00:00:00Z",
  },
];

test("delete action shows visible progress while a large folder is being removed", async ({ mount, page }) => {
  const installTauriMocks = ({ entries }: { entries: Array<Record<string, unknown>> }) => {
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
          if (cmd === "list_entries") return entries;
          if (cmd === "delete_path") return new Promise(() => undefined);
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

  await page.addInitScript(installTauriMocks, { entries });
  await page.evaluate(installTauriMocks, { entries });

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <DeleteProgressHarness />
    </div>,
  );

  await page.getByRole("option", { name: "demo" }).click();
  await page.keyboard.press("Delete");
  await page.getByRole("button", { name: "Delete" }).click();

  await expect(page.getByLabel("Deletion in progress")).toBeVisible();
  await expect(page.getByText("Deleting 1 item")).toBeVisible();
  await expect(page.getByText(/Removing demo/)).toBeVisible();
  await expect(page.getByText("Large folders can take a while. Keep this window open until deletion finishes.")).toBeVisible();

  await expect(page).toHaveScreenshot("delete-progress.png");
});
