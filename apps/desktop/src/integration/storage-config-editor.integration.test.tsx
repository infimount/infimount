import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StorageConfigEditorDialog } from "@/components/StorageConfigEditorDialog";

describe("StorageConfigEditorDialog integration", () => {
  it("loads, formats, and saves the combined registry JSON", async () => {
    const onLoad = vi.fn().mockResolvedValue('[{"name":"Local","backend":"local","config":{"root":"/tmp"}}]');
    const onSave = vi.fn().mockResolvedValue(undefined);

    render(
      <StorageConfigEditorDialog
        open
        onOpenChange={() => undefined}
        onLoad={onLoad}
        onSave={onSave}
      />,
    );

    await waitFor(() => {
      expect(onLoad).toHaveBeenCalled();
    });

    fireEvent.click(screen.getByRole("button", { name: /Format JSON/i }));
    fireEvent.click(screen.getByRole("button", { name: /Apply JSON/i }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith(`[
  {
    "name": "Local",
    "backend": "local",
    "config": {
      "root": "/tmp"
    }
  }
]`);
    });
  });
});
