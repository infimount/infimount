import { expect, test } from "@playwright/experimental-ct-react";

import { UploadFlowHarness } from "@/test/UploadFlowHarness";

const baseEntries = [
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
];

type WriteCall = { sourceId?: unknown; path?: unknown; dataLength: number };

const installTauriMocks = ({ entries, keepWritesPending }: { entries: Array<Record<string, unknown>>; keepWritesPending: boolean }) => {
  Object.defineProperty(window, "__infimountWrites", {
    configurable: true,
    value: [] as WriteCall[],
  });
  const uploadBytes = new Map<string, number>();

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
        if (cmd === "begin_file_upload") {
          const uploadId = `upload-${uploadBytes.size + 1}`;
          uploadBytes.set(uploadId, 0);
          return uploadId;
        }
        if (cmd === "append_file_upload_chunk") {
          const uploadId = String(args?.uploadId);
          const chunkLength = Array.isArray(args?.data) ? args.data.length : 0;
          uploadBytes.set(uploadId, (uploadBytes.get(uploadId) ?? 0) + chunkLength);
          return null;
        }
        if (cmd === "finish_file_upload") {
          const uploadId = String(args?.uploadId);
          const writes = (window as unknown as { __infimountWrites: WriteCall[] }).__infimountWrites;
          writes.push({
            sourceId: args?.sourceId,
            path: args?.targetPath,
            dataLength: uploadBytes.get(uploadId) ?? 0,
          });
          if (keepWritesPending) return new Promise(() => undefined);
          uploadBytes.delete(uploadId);
          return null;
        }
        if (cmd === "cancel_file_upload") {
          uploadBytes.delete(String(args?.uploadId));
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

test("upload action shows real progress while a file is being written", async ({ mount, page }) => {
  const mockOptions = { entries: baseEntries, keepWritesPending: true };
  await page.addInitScript(installTauriMocks, mockOptions);
  await page.evaluate(installTauriMocks, mockOptions);

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <UploadFlowHarness />
    </div>,
  );

  await page.locator("#file-upload").setInputFiles({
    name: "new-upload.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("fresh upload"),
  });

  await expect(page.getByLabel("Upload in progress")).toBeVisible();
  await expect(page.getByText("Uploading 1 file")).toBeVisible();
  await expect(page.getByText(/Writing new-upload\.txt/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel remaining" })).toBeVisible();
  await expect(page).toHaveScreenshot("upload-progress.png");
});

test("upload conflict dialog exposes discard, keep both, and overwrite choices", async ({ mount, page }) => {
  const mockOptions = { entries: baseEntries, keepWritesPending: true };
  await page.addInitScript(installTauriMocks, mockOptions);
  await page.evaluate(installTauriMocks, mockOptions);

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <UploadFlowHarness />
    </div>,
  );

  await page.locator("#file-upload").setInputFiles({
    name: "report.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("replacement report"),
  });

  await expect(page.getByText("Upload existing files?")).toBeVisible();
  await expect(page.getByText("One or more files already exist in")).toBeVisible();
  await expect(page.getByRole("button", { name: "Discard existing" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Keep both" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Overwrite" })).toBeVisible();
  await expect(page).toHaveScreenshot("upload-conflict-dialog.png");

  await page.getByRole("button", { name: "Keep both" }).click();
  await expect
    .poll(async () => page.evaluate(() => (window as unknown as { __infimountWrites: WriteCall[] }).__infimountWrites))
    .toEqual([{ sourceId: "local", path: "/report copy.txt", dataLength: "replacement report".length }]);
});
