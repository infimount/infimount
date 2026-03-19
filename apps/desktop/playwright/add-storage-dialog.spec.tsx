import { expect, test } from "@playwright/experimental-ct-react";

import { MockedAddStorageDialog } from "@/test/MockedAddStorageDialog";

test("renders and submits the add storage dialog with mocked handlers", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <MockedAddStorageDialog />
    </div>,
  );

  await expect(page.getByText("Add New Storage")).toBeVisible();
  await page.locator("#storage-name").fill("Design Docs");
  await page.locator("#storage-field-root").fill("/Users/demo/Documents/design");
  await expect(page).toHaveScreenshot("add-storage-dialog.png");

  await page.getByRole("button", { name: "Add Storage" }).click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __PLAYWRIGHT_ADD_STORAGE_RESULT__?: unknown;
            }
          ).__PLAYWRIGHT_ADD_STORAGE_RESULT__ ?? null,
      ),
    )
    .toEqual({
      name: "Design Docs",
      backend: "local",
      config: {
        root: "/Users/demo/Documents/design",
      },
      enabled: true,
      mcpExposed: true,
      readOnly: false,
    });
});
