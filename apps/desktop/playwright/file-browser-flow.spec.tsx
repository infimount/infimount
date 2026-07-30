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

const installTauriMocks = ({ entriesByPath, failRoot = false, transferConflict = false }: { entriesByPath: Record<string, Array<Record<string, unknown>>>; failRoot?: boolean; transferConflict?: boolean }) => {
  const defaultPlan = {
    operation: "copy",
    conflictPolicy: "fail",
    entries: [
      {
        sourcePath: "/report.txt",
        destinationPath: "/report.txt",
        action: "create",
        isDir: false,
        size: 1200,
      },
    ],
    summary: { create: 1, overwrite: 0, skip: 0, rename: 0, noop: 0, conflict: 0, totalItems: 1, totalBytes: 1200 },
  };
  let transferCalls = 0;
  Object.defineProperty(window, "__infimountTransferPolicies", {
    configurable: true,
    value: [] as string[],
  });
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
        if (cmd === "list_entries" || cmd === "list_entries_page") {
          if (failRoot) {
            throw { code: "IO_ERROR", message: "backend request failed" };
          }
          const path = typeof args?.path === "string" ? args.path : "/";
          const entries = entriesByPath[path] ?? [];
          return cmd === "list_entries_page"
            ? { entries, nextCursor: null, truncated: false }
            : entries;
        }
        if (cmd === "plan_transfer_entries") return { ...defaultPlan, conflictPolicy: args?.conflictPolicy ?? "fail" };
        if (cmd === "transfer_entries") {
          transferCalls += 1;
          const policy = typeof args?.conflictPolicy === "string" ? args.conflictPolicy : "";
          (window as unknown as { __infimountTransferPolicies: string[] }).__infimountTransferPolicies.push(policy);
          if (transferConflict && transferCalls === 1) {
            throw { code: "ALREADY_EXISTS", message: "already exists" };
          }
          return null;
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

async function mountFileBrowser(mount: Parameters<Parameters<typeof test>[1]>[0]["mount"], page: Parameters<Parameters<typeof test>[1]>[0]["page"], options: { initialEntries?: typeof entriesByPath; failRoot?: boolean; transferConflict?: boolean } = {}) {
  const mockOptions = {
    entriesByPath: options.initialEntries ?? entriesByPath,
    failRoot: options.failRoot ?? false,
    transferConflict: options.transferConflict ?? false,
  };
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

test("resolves copy paste conflicts from the FileBrowser flow", async ({ mount, page }) => {
  await mountFileBrowser(mount, page, { transferConflict: true });

  await page.getByRole("option", { name: "report.txt" }).click();
  await page.keyboard.press("Control+C");
  await page.keyboard.press("Control+V");

  await expect(page.getByText("Item already exists")).toBeVisible();
  await expect(page.getByRole("button", { name: "Overwrite" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Keep both" })).toBeVisible();
  await page.getByRole("button", { name: "Keep both" }).click();

  await expect
    .poll(async () => page.evaluate(() => (window as unknown as { __infimountTransferPolicies: string[] }).__infimountTransferPolicies))
    .toEqual(["fail", "rename"]);
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
