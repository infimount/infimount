import { expect, test } from "@playwright/experimental-ct-react";

import { TransferQueueHarness } from "@/test/TransferQueueHarness";

type TransferMockMode = "pending" | "cancel" | "fail-once";

type TransferState = {
  planCalls: number;
  transferCalls: number;
  cancelCalls: string[];
  rejectActive?: (error: Error) => void;
};

const installTauriMocks = ({ mode }: { mode: TransferMockMode }) => {
  const defaultPlan = {
    entries: [
      {
        sourcePath: "/report.txt",
        destinationPath: "/incoming/report.txt",
        action: "create",
        isDir: false,
        size: 1200,
      },
    ],
    summary: {
      totalItems: 1,
      create: 1,
      overwrite: 0,
      rename: 0,
      skip: 0,
      conflict: 0,
    },
    hasConflicts: false,
  };
  const state: TransferState = {
    planCalls: 0,
    transferCalls: 0,
    cancelCalls: [],
  };
  Object.defineProperty(window, "__infimountTransferState", {
    configurable: true,
    value: state,
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
        if (cmd === "plugin:event|listen") return args?.handler ?? 1;
        if (cmd === "plugin:event|unlisten") return null;
        if (cmd === "plugin:app|version") return "0.7.1";
        if (cmd.includes("updater") || cmd.includes("window")) return null;
        if (cmd === "plan_transfer_entries") {
          state.planCalls += 1;
          if (mode === "fail-once" && state.planCalls === 1) {
            throw "Destination already exists";
          }
          return defaultPlan;
        }
        if (cmd === "transfer_entries") {
          state.transferCalls += 1;
          if (mode === "pending" || mode === "fail-once") return new Promise(() => undefined);
          if (mode === "cancel") {
            return new Promise((_resolve, reject) => {
              state.rejectActive = reject;
            });
          }
        }
        if (cmd === "cancel_transfer_job") {
          const jobId = typeof args?.jobId === "string" ? args.jobId : "";
          state.cancelCalls.push(jobId);
          state.rejectActive?.(new Error("Transfer cancelled"));
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

async function mountTransferQueue(mount: Parameters<Parameters<typeof test>[1]>[0]["mount"], page: Parameters<Parameters<typeof test>[1]>[0]["page"], mode: TransferMockMode) {
  const mockOptions = { mode };
  await page.addInitScript(installTauriMocks, mockOptions);
  await page.evaluate(installTauriMocks, mockOptions);

  await mount(
    <div className="h-screen w-screen bg-background p-4">
      <TransferQueueHarness />
    </div>,
  );
}

test("transfer queue shows visible running progress", async ({ mount, page }) => {
  await mountTransferQueue(mount, page, "pending");

  await page.getByRole("button", { name: "Queue copy" }).click();

  await expect(page.getByLabel("Transfer queue")).toBeVisible();
  await expect(page.getByText("Transfer queue")).toBeVisible();
  await expect(page.getByText("Running").first()).toBeVisible();
  await expect(page.getByText("Copy 1 item")).toBeVisible();
  await expect(page.getByText("Local Docs → Archive Bucket · /incoming")).toBeVisible();
  await expect(page.getByText("Dry-run: 1 create")).toBeVisible();
  await expect(page.getByText("Current: Starting transfer...")).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel active transfer" })).toBeVisible();
  await expect(page).toHaveScreenshot("transfer-queue-running.png");
});

test("transfer queue can cancel an active transfer", async ({ mount, page }) => {
  await mountTransferQueue(mount, page, "cancel");

  await page.getByRole("button", { name: "Queue copy" }).click();
  await expect(page.getByRole("button", { name: "Cancel active transfer" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel active transfer" }).click();

  await expect(page.getByText("Cancelled")).toHaveCount(0);
  await expect(page.getByText("No active transfers.")).toBeVisible();
  await expect
    .poll(async () => page.evaluate(() => (window as unknown as { __infimountTransferState: TransferState }).__infimountTransferState.cancelCalls.length))
    .toBe(1);
});

test("transfer queue can retry a failed transfer", async ({ mount, page }) => {
  await mountTransferQueue(mount, page, "fail-once");

  await page.getByRole("button", { name: "Queue copy" }).click();
  await expect(page.getByText("Failed").first()).toBeVisible();
  await expect(page.getByText("Destination already exists")).toBeVisible();

  await page.getByRole("button", { name: "Retry transfer" }).click();

  await expect(page.getByText("Running").first()).toBeVisible();
  await expect(page.getByText("Dry-run: 1 create")).toBeVisible();
  await expect
    .poll(async () => page.evaluate(() => (window as unknown as { __infimountTransferState: TransferState }).__infimountTransferState.planCalls))
    .toBe(2);
});
