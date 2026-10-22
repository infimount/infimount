import { expect, test } from "@playwright/experimental-ct-react";

import { StorageOnlyOnboardingHarness } from "@/test/StorageOnlyOnboardingHarness";

async function failAllBackendInvocations(page: Parameters<Parameters<typeof test>[1]>[0]["page"]) {
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {
        invoke: async (cmd: string) => {
          throw new Error(`Unexpected Tauri call during storage-only onboarding: ${cmd}`);
        },
        transformCallback: () => 1,
        unregisterCallback: () => undefined,
      },
    });
  });
}

test("opens a configured file browser without MCP setup or any backend calls", async ({
  mount,
  page,
}) => {
  await failAllBackendInvocations(page);
  await mount(<StorageOnlyOnboardingHarness />);

  const browse = page.getByRole("button", { name: "Browse storage only" });
  await expect(browse).toBeVisible();
  await expect(browse).toHaveClass(/bg-primary/);
  await browse.click();

  await expect(page.getByTestId("storage-only-state")).toHaveText("File browser ready");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("new users are guided to add storage before optional agent steps", async ({
  mount,
  page,
}) => {
  await failAllBackendInvocations(page);
  await mount(<StorageOnlyOnboardingHarness storagesCount={0} />);

  const proceed = page.getByRole("button", { name: "Continue" });
  await expect(proceed).toHaveClass(/bg-primary/);
  await proceed.click();

  await expect(page.getByRole("button", { name: "Add and validate storage" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toBeDisabled();
  await expect(page.getByRole("heading", { name: "Prepare agent access" })).toHaveCount(0);
});
