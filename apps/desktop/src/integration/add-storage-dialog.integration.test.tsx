import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AddStorageDialog } from "@/components/AddStorageDialog";
import type { StorageKindSchema } from "@/lib/api";

const schemas: StorageKindSchema[] = [
  {
    id: "local-fs",
    label: "Local File System",
    kind: "local",
    fields: [
      {
        name: "rootPath",
        label: "Root Folder Path",
        input_type: "text",
        required: true,
        secret: false,
      },
    ],
  },
  {
    id: "gcs",
    label: "Google Cloud Storage",
    kind: "gcs",
    fields: [
      {
        name: "bucket",
        label: "Bucket",
        input_type: "text",
        required: true,
        secret: false,
      },
      {
        name: "credential",
        label: "Service Account JSON",
        input_type: "textarea",
        required: false,
        secret: true,
      },
    ],
  },
];

describe("AddStorageDialog integration", () => {
  it("builds a storage draft from schema-driven fields", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);

    render(
      <AddStorageDialog
        open
        onOpenChange={() => undefined}
        onAdd={onAdd}
        loadSchemas={async () => schemas}
      />,
    );

    await screen.findByText("Backend Fields");

    fireEvent.change(screen.getByLabelText("Storage Name"), {
      target: { value: "Downloads" },
    });
    fireEvent.change(screen.getByLabelText("Root Folder Path *"), {
      target: { value: "~/Downloads" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "Downloads",
        backend: "local",
        config: { rootPath: "~/Downloads" },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
    });
  });

  it("switches schemas and keeps secret fields masked until revealed", async () => {
    render(
      <AddStorageDialog
        open
        onOpenChange={() => undefined}
        loadSchemas={async () => schemas}
      />,
    );

    await screen.findByText("Backend Fields");
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.click(await screen.findByRole("option", { name: /Google Cloud Storage/i }));

    await screen.findByLabelText("Bucket *");
    expect(screen.getByRole("button", { name: /(Mask|Reveal) Secrets/i })).toBeInTheDocument();
    const credentialField = screen.getByLabelText("Service Account JSON");
    expect(credentialField).toHaveAttribute("rows", "6");
  });
});
