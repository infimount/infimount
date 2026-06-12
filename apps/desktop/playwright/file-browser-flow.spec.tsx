import { expect, test } from "@playwright/experimental-ct-react";

import { UploadFlowHarness } from "@/test/UploadFlowHarness";

const entriesByPath: Record<string, Array<Record<string, unknown>>> = {
  "/": [
    {
      path: "/docs",
      name: "docs",
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
    {
      path: "/photo.jpg",
      name: "photo.jpg",
      is_dir: false,
      size: 2048,
      modified_at: "2026-01-02T00:00:00Z",
    },
  ],
  "/docs": [
    {
      path: "/docs/guide.md",
      name: "guide.md",
      is_dir: false,
      size: 4096,
      modified_at: "2026-01-03T00:00:00Z",
    },
  ],
  "/empty": [],
};

const installTauriMocks = ({ entriesByPath, failRoot = false }: { entriesByPath: Record<string, Array<Record<string, unknown>>>; failRoot?: boolean }) => {
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
        if (cmd === "plugin:app|version") return "0.7.1";
        if (cmd === "plugin:event|listen") return args?.handler ?? 1;
        if (cmd === "plugin:event|unlisten") return null;
        if (cmd.includes("updater") || cmd.includes("window")) return null;
        if (cmd === "list_entries") {
          if (failRoot) {
            throw { code: "IO_ERROR", message: "backend request failed" };
          }
          const path = typeof args?.path === "string" ? args.path : "/";
          return entriesByPath[path] ?? [];
        }
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

async function mountFileBrowser(mount: Parameters<Parameters<typeof test>[1]>[0]["mount"], page: Parameters<Parameters<typeof test>[1]>[0]["page"], options: { initialEntries?: typeof entriesByPath; failRoot?: boolean } = {}) {
  const mockOptions = { entriesByPath: options.initialEntries ?? entriesByPath, failRoot: options.failRoot ?? false };
  await page.addInitScript(installTauriMocks, mockOptions);
  await page.evaluate(installTauriMocks, mockOptions);

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <UploadFlowHarness />
    </div>,
  );
}

test("browses into a folder and updates the visible location", async ({ mount, page }) => {
  await mountFileBrowser(mount, page);

  await expect(page.getByRole("option", { name: "docs" })).toBeVisible();
  await page.getByRole("option", { name: "docs" }).dblclick();

  await expect(page.getByText("guide.md")).toBeVisible();
  await expect(page.getByText("docs").first()).toBeVisible();
  await expect(page.getByRole("option", { name: "report.txt" })).toHaveCount(0);
});

test("filters files through search and switches to list view", async ({ mount, page }) => {
  await mountFileBrowser(mount, page);

  await page.getByPlaceholder("Search...").fill("photo");
  await expect(page.getByRole("option", { name: "photo.jpg" })).toBeVisible();
  await expect(page.getByRole("option", { name: "report.txt" })).toHaveCount(0);

  await page.getByTitle("Switch to list view").click();
  await expect(page.getByRole("columnheader", { name: "Name" })).toBeVisible();
  await expect(page.getByRole("columnheader", { name: "Modified" })).toBeVisible();
  await expect(page.getByRole("row", { name: /photo\.jpg/ })).toBeVisible();
  await expect(page).toHaveScreenshot("file-browser-search-list.png");
});

test("shows a friendly empty folder state", async ({ mount, page }) => {
  await mountFileBrowser(mount, page, { initialEntries: { "/": [] } });

  await expect(page.getByText("This folder is empty")).toBeVisible();
  await expect(page.getByText("Drop files here to upload, or navigate to another folder.")).toBeVisible();
});

test("shows a friendly load error state with retry", async ({ mount, page }) => {
  await mountFileBrowser(mount, page, { failRoot: true });

  await expect(page.getByText("Network issue")).toBeVisible();
  await expect(page.getByText("Unable to reach the storage service. Verify network/VPN settings.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
});
