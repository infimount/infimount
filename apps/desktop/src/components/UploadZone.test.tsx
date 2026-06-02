import { createRef } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { UploadZone, type UploadZoneRef } from "./UploadZone";

describe("UploadZone", () => {
  it("renders the drag affordance only while dragging", () => {
    const { rerender } = render(<UploadZone onUpload={vi.fn()} isDragging={false} />);

    expect(screen.queryByText("Drop files here to upload")).not.toBeInTheDocument();

    rerender(<UploadZone onUpload={vi.fn()} isDragging />);

    expect(screen.getByText("Drop files here to upload")).toBeInTheDocument();
  });

  it("accepts file input selection and forwards files to onUpload immediately", () => {
    const onUpload = vi.fn();

    const { container } = render(<UploadZone onUpload={onUpload} isDragging={false} />);
    const input = container.querySelector("#file-upload") as HTMLInputElement | null;
    expect(input).toBeTruthy();

    const file = new File(["hello"], "hello.txt", { type: "text/plain" });
    fireEvent.change(input!, { target: { files: [file] } });

    expect(onUpload).toHaveBeenCalledTimes(1);
    const uploaded = onUpload.mock.calls[0][0];
    expect(uploaded).toHaveLength(1);
    expect(uploaded[0].name).toBe("hello.txt");
    expect(input!.value).toBe("");
    expect(screen.queryByText("Uploading Files")).not.toBeInTheDocument();
  });

  it("uses relative paths for directory picks and ignores empty selections", () => {
    const onUpload = vi.fn();

    const { container } = render(<UploadZone onUpload={onUpload} />);
    const input = container.querySelector("#file-upload") as HTMLInputElement;

    fireEvent.change(input, { target: { files: [] } });
    expect(onUpload).not.toHaveBeenCalled();

    const file = new File(["hello"], "hello.txt", { type: "text/plain" });
    Object.defineProperty(file, "webkitRelativePath", {
      value: "folder/hello.txt",
      configurable: true,
    });

    fireEvent.change(input, { target: { files: [file] } });

    expect(onUpload).toHaveBeenCalledTimes(1);
    expect(onUpload.mock.calls[0][0][0].name).toBe("folder/hello.txt");
  });

  it("exposes imperative uploads", () => {
    const onUpload = vi.fn();
    const ref = createRef<UploadZoneRef>();
    const file = {
      name: "manual.txt",
      arrayBuffer: vi.fn(async () => new ArrayBuffer(0)),
    };

    render(<UploadZone ref={ref} onUpload={onUpload} />);

    ref.current?.handleFiles([file]);

    expect(onUpload).toHaveBeenCalledWith([file]);
  });
});
