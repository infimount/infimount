import { expect, test } from "@playwright/experimental-ct-react";

import { MockedFilePreviewPanel } from "@/test/MockedFilePreviewPanel";

test("renders the file preview panel with mocked file contents", async ({ mount, page }) => {
  await mount(
    <div className="h-[720px] w-[420px] bg-background p-6">
      <div className="h-full overflow-hidden rounded-2xl border border-border bg-card">
        <MockedFilePreviewPanel />
      </div>
    </div>,
  );

  await expect(page.getByText("Infimount preview content.")).toBeVisible();
  await expect(page).toHaveScreenshot("file-preview-panel.png");
});
