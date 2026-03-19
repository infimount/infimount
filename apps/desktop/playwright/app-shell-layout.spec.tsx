import { expect, test } from "@playwright/experimental-ct-react";

import { StorageSidebar } from "@/components/StorageSidebar";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import type { StorageConfig } from "@/types/storage";

const storages: StorageConfig[] = [
  {
    id: "downloads",
    name: "Downloads",
    backend: "local",
    type: "local-fs",
    config: { root: "/home/test/Downloads" },
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    connected: true,
    createdAt: "2026-03-01T00:00:00Z",
    updatedAt: "2026-03-01T00:00:00Z",
  },
  {
    id: "s3",
    name: "S3 Bucket",
    backend: "s3",
    type: "aws-s3",
    config: { bucket: "demo-bucket", region: "us-east-1" },
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    connected: true,
    createdAt: "2026-03-01T00:00:00Z",
    updatedAt: "2026-03-01T00:00:00Z",
  },
  {
    id: "gcs",
    name: "Google Storage",
    backend: "gcs",
    type: "gcs",
    config: { bucket: "demo-gcs" },
    enabled: true,
    mcpExposed: true,
    readOnly: true,
    connected: true,
    createdAt: "2026-03-01T00:00:00Z",
    updatedAt: "2026-03-01T00:00:00Z",
  },
];

test("renders the desktop shell with a visible sidebar", async ({ mount, page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {
        invoke: async (cmd: string) => {
          if (cmd === "plugin:app|version") {
            return "0.2.1";
          }
          if (cmd.includes("updater")) {
            return null;
          }
          if (cmd.includes("window")) {
            return false;
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
  });

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <div className="flex h-full w-full overflow-hidden rounded-[12px] border border-border/40 bg-background">
        <ResizablePanelGroup direction="horizontal">
          <ResizablePanel
            className="hidden md:block transition-all duration-200"
            defaultSize="20%"
            minSize="15%"
            maxSize="40%"
          >
            <div data-testid="shell-sidebar" className="h-full">
              <StorageSidebar
                storages={storages}
                selectedStorage="downloads"
                onSelectStorage={() => undefined}
                onAddStorage={() => undefined}
                onEditStorage={() => undefined}
                onDeleteStorage={() => undefined}
                onRefreshStorage={() => undefined}
                onImportStorages={() => undefined}
                onEditStorageConfig={() => undefined}
                onExportStorages={() => undefined}
                onOpenMcpSettings={() => undefined}
                isLoading={false}
              />
            </div>
          </ResizablePanel>
          <ResizableHandle className="hidden md:flex w-px flex-col items-center justify-center bg-transparent group/handle relative z-10">
            <div className="absolute inset-y-0 -left-1 -right-1 z-50 cursor-col-resize" />
            <div className="h-full w-[1px] bg-border/40 transition-colors group-hover/handle:bg-primary/40" />
          </ResizableHandle>
          <ResizablePanel className="flex-1 overflow-hidden">
            <div className="flex h-full items-center justify-center bg-white text-sm text-muted-foreground">
              Main content
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </div>,
  );

  const sidebar = page.getByTestId("shell-sidebar");
  await expect(sidebar).toBeVisible();

  const box = await sidebar.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.width).toBeGreaterThan(180);

  await expect(page).toHaveScreenshot("app-shell-layout.png");
});
