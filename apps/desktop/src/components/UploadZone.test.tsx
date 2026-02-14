import { render, fireEvent, act } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UploadZone } from "./UploadZone";

describe("UploadZone", () => {
  it("accepts file input selection and forwards files to onUpload", async () => {
    vi.useFakeTimers();
    const onUpload = vi.fn();

    const { container } = render(<UploadZone onUpload={onUpload} isDragging={false} />);
    const input = container.querySelector("#file-upload") as HTMLInputElement | null;
    expect(input).toBeTruthy();

    const file = new File(["hello"], "hello.txt", { type: "text/plain" });
    fireEvent.change(input!, { target: { files: [file] } });

    await act(async () => {
      vi.advanceTimersByTime(2600);
    });

    expect(onUpload).toHaveBeenCalledTimes(1);
    const uploaded = onUpload.mock.calls[0][0];
    expect(uploaded).toHaveLength(1);
    expect(uploaded[0].name).toBe("hello.txt");
    expect(input!.value).toBe("");
    vi.useRealTimers();
  });
});
