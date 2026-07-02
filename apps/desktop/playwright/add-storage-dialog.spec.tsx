import { expect, test } from "@playwright/experimental-ct-react";

import { MockedAddStorageDialog } from "@/test/MockedAddStorageDialog";
import { MockedOAuthAddStorageDialog } from "@/test/MockedOAuthAddStorageDialog";

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
      mcpExposed: false,
      readOnly: false,
    });
});

test("renders Google Drive OAuth connect states without visible secrets", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <MockedOAuthAddStorageDialog provider="gdrive" />
    </div>,
  );

  await expect(page.getByText("Connect Google Drive")).toBeVisible();
  await expect(page.getByText(/local loopback callback/)).toBeVisible();
  await expect(page).toHaveScreenshot("add-storage-oauth-google-drive.png");

  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByText(/OAuth connected/)).toBeVisible();
  await expect(page.getByLabel("Access Token")).toHaveAttribute("type", "password");
  await expect(page.getByText("playwright-access-token")).toHaveCount(0);
  await expect(page).toHaveScreenshot("add-storage-oauth-google-drive-success.png");
});

test("renders OAuth waiting state", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <MockedOAuthAddStorageDialog provider="gdrive" mode="waiting" />
    </div>,
  );

  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("button", { name: /Waiting for browser/ })).toBeVisible();
  await expect(page.getByText(/Opening your browser/)).toBeVisible();
  await expect(page).toHaveScreenshot("add-storage-oauth-waiting.png");
});

test("renders OAuth error state", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <MockedOAuthAddStorageDialog provider="gdrive" mode="error" />
    </div>,
  );

  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByText("OAuth authorization failed without exposing tokens.")).toBeVisible();
  await expect(page).toHaveScreenshot("add-storage-oauth-error.png");
});

test("renders Microsoft OneDrive OAuth connection options", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <MockedOAuthAddStorageDialog provider="onedrive" />
    </div>,
  );

  await expect(page.getByText("Connect Microsoft OneDrive")).toBeVisible();
  await expect(page.getByLabel("Enable Versioning")).toBeChecked();
  await expect(page).toHaveScreenshot("add-storage-oauth-onedrive.png");
});
