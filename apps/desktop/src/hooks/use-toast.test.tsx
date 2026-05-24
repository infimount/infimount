import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi, afterEach } from "vitest";

import { reducer, toast, useToast } from "./use-toast";

describe("use-toast reducer", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("adds, updates, dismisses, and removes toast state", () => {
    vi.useFakeTimers();

    const first = { id: "1", title: "First", open: true };
    const second = { id: "2", title: "Second", open: true };

    const added = reducer({ toasts: [] }, { type: "ADD_TOAST", toast: first });
    expect(added.toasts).toEqual([first]);

    const limited = reducer(added, { type: "ADD_TOAST", toast: second });
    expect(limited.toasts).toEqual([second]);

    const updated = reducer(limited, {
      type: "UPDATE_TOAST",
      toast: { id: "2", title: "Updated" },
    });
    expect(updated.toasts[0]).toMatchObject({ id: "2", title: "Updated", open: true });

    const dismissed = reducer(updated, { type: "DISMISS_TOAST", toastId: "2" });
    expect(dismissed.toasts[0]).toMatchObject({ id: "2", open: false });

    const removedOne = reducer(dismissed, { type: "REMOVE_TOAST", toastId: "2" });
    expect(removedOne.toasts).toEqual([]);

    const dismissedAll = reducer(
      { toasts: [first, second] },
      { type: "DISMISS_TOAST" },
    );
    expect(dismissedAll.toasts.every((item) => item.open === false)).toBe(true);

    const removedAll = reducer(dismissedAll, { type: "REMOVE_TOAST" });
    expect(removedAll.toasts).toEqual([]);
  });
});

function ToastProbe() {
  const { toasts, dismiss } = useToast();

  return (
    <div>
      <output data-testid="toast-count">{toasts.length}</output>
      {toasts.map((item) => (
        <div key={item.id}>
          <span>{String(item.title)}</span>
          <span>{item.open ? "open" : "closed"}</span>
          <button type="button" onClick={() => dismiss(item.id)}>
            dismiss {item.id}
          </button>
        </div>
      ))}
    </div>
  );
}

describe("useToast", () => {
  it("publishes toast actions to hook subscribers", async () => {
    render(<ToastProbe />);

    let created!: ReturnType<typeof toast>;
    act(() => {
      created = toast({ title: "Saved" });
    });

    expect(screen.getByText("Saved")).toBeInTheDocument();
    expect(screen.getByTestId("toast-count")).toHaveTextContent("1");

    act(() => {
      created.update({ id: created.id, title: "Renamed" });
    });
    expect(screen.getByText("Renamed")).toBeInTheDocument();

    act(() => {
      created.dismiss();
    });
    expect(screen.getByText("closed")).toBeInTheDocument();
  });
});
