import { expect, test } from "@playwright/experimental-ct-react";

import { UploadFlowHarness } from "@/test/UploadFlowHarness";

type PreviewState = {
  readFileCalls: string[];
  downloadedNames: string[];
};

const capabilities = {
  list: true,
  read: true,
  write: true,
  delete: true,
  copy: false,
  rename: false,
  presign_read: false,
  write_can_empty: true,
  write_can_append: false,
  write_can_multi: false,
  write_with_content_type: false,
  write_with_cache_control: false,
  write_with_content_disposition: false,
  write_with_content_encoding: false,
  write_with_user_metadata: false,
  stat_with_if_match: false,
  stat_with_if_none_match: false,
  read_with_if_match: false,
  read_with_if_none_match: false,
  read_with_override_cache_control: false,
  read_with_override_content_disposition: false,
  read_with_override_content_type: false,
  batch: false,
  blocking: false,
  list_with_limit: false,
  list_with_start_after: false,
  list_with_recursive: true,
  list_with_versions: false,
  presign_stat: false,
  presign_write: false,
  shared: false,
};

const installTauriMocks = ({ failRead = false }: { failRead?: boolean }) => {
  const previewBytes = Array.from(new TextEncoder().encode("# Report\n\nPreview from FileBrowser flow."));
  const entries = [
    {
      path: "/report.md",
      name: "report.md",
      is_dir: false,
      size: 1200,
      modified_at: "2026-01-01T00:00:00Z",
    },
  ];
  Object.defineProperty(window, "__infimountPreviewState", {
    configurable: true,
    value: { readFileCalls: [], downloadedNames: [] } as PreviewState,
  });
  URL.createObjectURL = () => "blob:report-download";
  URL.revokeObjectURL = () => undefined;
  HTMLAnchorElement.prototype.click = function click() {
    (window as unknown as { __infimountPreviewState: PreviewState }).__infimountPreviewState.downloadedNames.push(this.download);
  };

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
        if (cmd === "list_entries") return entries;
        if (cmd === "list_entries_page") return { entries, nextCursor: null, truncated: false };
        if (cmd === "get_storage_capabilities") return capabilities;
        if (cmd === "download_file_to_downloads") {
          const path = typeof args?.path === "string" ? args.path : "";
          const name = path.split("/").filter(Boolean).pop() ?? "download";
          (window as unknown as { __infimountPreviewState: PreviewState }).__infimountPreviewState.downloadedNames.push(name);
          return { fileName: name, bytes: previewBytes.length };
        }
        if (cmd === "read_file" || cmd === "read_file_range") {
          const path = typeof args?.path === "string" ? args.path : "";
          (window as unknown as { __infimountPreviewState: PreviewState }).__infimountPreviewState.readFileCalls.push(path);
          if (failRead) throw "Read failed";
          return cmd === "read_file_range"
            ? { totalSize: previewBytes.length, offset: 0, bytes: previewBytes, truncated: false }
            : previewBytes;
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

async function mountPreviewHarness(mount: Parameters<Parameters<typeof test>[1]>[0]["mount"], page: Parameters<Parameters<typeof test>[1]>[0]["page"], options: { failRead?: boolean } = {}) {
  await page.addInitScript(installTauriMocks, options);
  await page.evaluate(installTauriMocks, options);

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <UploadFlowHarness />
    </div>,
  );
}

test("opens preview from FileBrowser and downloads the file", async ({ mount, page }) => {
  await mountPreviewHarness(mount, page);

  await page.getByRole("option", { name: "report.md" }).dblclick();

  await expect(page.getByText("Preview from FileBrowser flow.")).toBeVisible();
  await expect(page.getByText("File Information")).toBeVisible();
  await page.getByRole("button", { name: "Download" }).click();

  await expect
    .poll(async () => page.evaluate(() => (window as unknown as { __infimountPreviewState: PreviewState }).__infimountPreviewState.downloadedNames))
    .toEqual(["report.md"]);
  await expect(page).toHaveScreenshot("file-browser-preview-open.png");
});

test("preview read failures show an inline error", async ({ mount, page }) => {
  await mountPreviewHarness(mount, page, { failRead: true });

  await page.getByRole("option", { name: "report.md" }).dblclick();

  await expect(page.getByText("Read failed")).toBeVisible();
  await expect(page.getByText("File Information")).toBeVisible();
});
