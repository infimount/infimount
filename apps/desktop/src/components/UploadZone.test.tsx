import { createRef } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { UploadZone, type UploadZoneRef } from "./UploadZone";

describe("UploadZone", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the drag affordance only while dragging", () => {
    const { rerender } = render(<UploadZone onUpload={vi.fn()} isDragging={false} />);

    expect(screen.queryByText("Drop files here to upload")).not.toBeInTheDocument();

    rerender(<UploadZone onUpload={vi.fn()} isDragging />);

    expect(screen.getByText("Drop files here to upload")).toBeInTheDocument();
  });

  it("accepts file input selection and forwards files to onUpload", async () => {
    vi.useFakeTimers();
    const onUpload = vi.fn();

    const { container } = render(<UploadZone onUpload={onUpload} isDragging={false} />);
    const input = container.querySelector("#file-upload") as HTMLInputElement | null;
    expect(input).toBeTruthy();

    const file = new File(["hello"], "hello.txt", { type: "text/plain" });
    fireEvent.change(input!, { target: { files: [file] } });

    expect(screen.getByText("Uploading Files")).toBeInTheDocument();
    expect(screen.getByText("hello.txt")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(2600);
    });

    expect(onUpload).toHaveBeenCalledTimes(1);
    const uploaded = onUpload.mock.calls[0][0];
    expect(uploaded).toHaveLength(1);
    expect(uploaded[0].name).toBe("hello.txt");
    expect(input!.value).toBe("");
    expect(screen.queryByText("Uploading Files")).not.toBeInTheDocument();
  });

  it("uses relative paths for directory picks and ignores empty selections", async () => {
    vi.useFakeTimers();
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

    await act(async () => {
      vi.advanceTimersByTime(2600);
    });

    expect(onUpload).toHaveBeenCalledTimes(1);
    expect(onUpload.mock.calls[0][0][0].name).toBe("folder/hello.txt");
  });

  it("exposes imperative uploads and lets in-flight rows be cancelled", async () => {
    vi.useFakeTimers();
    const onUpload = vi.fn();
    const ref = createRef<UploadZoneRef>();
    const file = {
      name: "manual.txt",
      arrayBuffer: vi.fn(async () => new ArrayBuffer(0)),
    };

    render(<UploadZone ref={ref} onUpload={onUpload} />);

    act(() => {
      ref.current?.handleFiles([file]);
    });

    expect(screen.getByText("manual.txt")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button"));
    expect(screen.queryByText("manual.txt")).not.toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(2600);
    });

    expect(onUpload).toHaveBeenCalledWith([file]);
    expect(screen.queryByText("Uploading Files")).not.toBeInTheDocument();
  });
});
