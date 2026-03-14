import { expect, test } from "@playwright/experimental-ct-react";

import { StorageConfigEditorDialog } from "@/components/StorageConfigEditorDialog";

test("renders the storage config editor dialog", async ({ mount, page }) => {
  await mount(
    <div className="min-h-screen bg-background p-8">
      <StorageConfigEditorDialog
        open
        onOpenChange={() => undefined}
        onLoad={async () => '[{"name":"Local","backend":"local","config":{"root":"/tmp"}}]'}
        onSave={async () => undefined}
      />
    </div>,
  );

  await expect(page.getByText("Edit Storage Config JSON")).toBeVisible();
  await expect(page.getByRole("button", { name: "Format JSON" })).toBeVisible();
  await expect(page).toHaveScreenshot("storage-config-editor-dialog.png");
});
