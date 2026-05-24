import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { StorageConfigEditorDialog } from "./StorageConfigEditorDialog";

vi.mock("@/components/JsonCodeEditor", () => ({
  JsonCodeEditor: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="JSON editor"
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  ),
}));

describe("StorageConfigEditorDialog", () => {
  const onOpenChange = vi.fn();
  const onLoad = vi.fn<() => Promise<string>>();
  const onSave = vi.fn<(json: string) => Promise<void>>();

  beforeEach(() => {
    vi.clearAllMocks();
    onLoad.mockResolvedValue('[{"id":"local","provider":"filesystem"}]');
    onSave.mockResolvedValue(undefined);
  });

  function renderDialog(open = true) {
    return render(
      <StorageConfigEditorDialog
        open={open}
        onOpenChange={onOpenChange}
        onLoad={onLoad}
        onSave={onSave}
      />,
    );
  }

  it("loads registry JSON when opened and supports manual reload", async () => {
    renderDialog();

    expect(await screen.findByDisplayValue('[{"id":"local","provider":"filesystem"}]')).toBeInTheDocument();
    expect(onLoad).toHaveBeenCalledTimes(1);

    onLoad.mockResolvedValueOnce('[{"id":"s3","provider":"s3"}]');
    fireEvent.click(screen.getByRole("button", { name: /reload/i }));

    expect(await screen.findByDisplayValue('[{"id":"s3","provider":"s3"}]')).toBeInTheDocument();
    expect(onLoad).toHaveBeenCalledTimes(2);
  });

  it("does not load while closed", () => {
    renderDialog(false);

    expect(onLoad).not.toHaveBeenCalled();
    expect(screen.queryByText("Edit Storage Config JSON")).not.toBeInTheDocument();
  });

  it("formats valid JSON and reports invalid JSON without keeping stale errors", async () => {
    renderDialog();
    const editor = await screen.findByLabelText("JSON editor");

    fireEvent.change(editor, { target: { value: '{"id":"local"}' } });
    fireEvent.click(screen.getByRole("button", { name: /format json/i }));

    expect(screen.getByLabelText("JSON editor")).toHaveValue(`{\n  "id": "local"\n}`);

    fireEvent.change(screen.getByLabelText("JSON editor"), { target: { value: "{" } });
    fireEvent.click(screen.getByRole("button", { name: /format json/i }));

    expect(screen.getByText(/expected property name|unexpected end/i)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("JSON editor"), { target: { value: "[]" } });
    expect(screen.queryByText(/expected property name|unexpected end/i)).not.toBeInTheDocument();
  });

  it("saves edited JSON and closes on success", async () => {
    renderDialog();
    const editor = await screen.findByLabelText("JSON editor");

    fireEvent.change(editor, { target: { value: '[{"id":"edited"}]' } });
    fireEvent.click(screen.getByRole("button", { name: /apply json/i }));

    await waitFor(() => expect(onSave).toHaveBeenCalledWith('[{"id":"edited"}]'));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("shows load and save failures", async () => {
    onLoad.mockRejectedValueOnce(new Error("load failed"));
    renderDialog();

    expect(await screen.findByText("load failed")).toBeInTheDocument();

    onSave.mockRejectedValueOnce(new Error("save failed"));
    fireEvent.click(screen.getByRole("button", { name: /apply json/i }));

    expect(await screen.findByText("save failed")).toBeInTheDocument();
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("closes without saving", async () => {
    renderDialog();
    await screen.findByLabelText("JSON editor");

    fireEvent.click(screen.getAllByRole("button", { name: /close/i })[0]);

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onSave).not.toHaveBeenCalled();
  });
});
