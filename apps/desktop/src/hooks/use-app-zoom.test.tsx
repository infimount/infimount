import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppZoomProvider, useAppZoom } from "./use-app-zoom";

function ZoomProbe() {
  const { zoom, setZoom, zoomIn, zoomOut, resetZoom } = useAppZoom();

  return (
    <div>
      <span data-testid="zoom">{zoom.toFixed(2)}</span>
      <button type="button" onClick={zoomIn}>Zoom in</button>
      <button type="button" onClick={zoomOut}>Zoom out</button>
      <button type="button" onClick={resetZoom}>Reset</button>
      <button type="button" onClick={() => setZoom(3)}>Set too high</button>
      <button type="button" onClick={() => setZoom(0.1)}>Set too low</button>
      <button type="button" onClick={() => setZoom(1.234)}>Set precise</button>
      <div data-testid="zoom-region" data-infimount-zoom-region="true">
        Zoom region
      </div>
      <div data-testid="outside-region">Outside region</div>
    </div>
  );
}

function renderZoomProbe() {
  return render(
    <AppZoomProvider>
      <ZoomProbe />
    </AppZoomProvider>,
  );
}

function dispatchKeyboardEvent(init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });

  act(() => {
    window.dispatchEvent(event);
  });

  return event;
}

function dispatchWheelEvent(target: Element, init: WheelEventInit) {
  const event = new WheelEvent("wheel", {
    bubbles: true,
    cancelable: true,
    ...init,
  });

  act(() => {
    target.dispatchEvent(event);
  });

  return event;
}

describe("useAppZoom", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requires an AppZoomProvider", () => {
    function MissingProviderProbe() {
      useAppZoom();
      return null;
    }

    vi.spyOn(console, "error").mockImplementation(() => undefined);

    expect(() => render(<MissingProviderProbe />)).toThrow(
      "useAppZoom must be used within AppZoomProvider",
    );
  });

  it("loads, normalizes, and persists zoom values", async () => {
    window.localStorage.setItem("infimount.zoom", "1.234");
    const { unmount } = renderZoomProbe();

    expect(screen.getByTestId("zoom")).toHaveTextContent("1.23");
    await waitFor(() => expect(window.localStorage.getItem("infimount.zoom")).toBe("1.23"));

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.33");
    expect(window.localStorage.getItem("infimount.zoom")).toBe("1.33");

    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.23");

    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");

    unmount();

    window.localStorage.setItem("infimount.zoom", "not a number");
    renderZoomProbe();

    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");
  });

  it("clamps direct zoom updates to the supported range", () => {
    renderZoomProbe();

    fireEvent.click(screen.getByRole("button", { name: "Set too high" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("2.00");

    fireEvent.click(screen.getByRole("button", { name: "Set too low" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("0.50");

    fireEvent.click(screen.getByRole("button", { name: "Set precise" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.23");
  });

  it("handles keyboard zoom shortcuts and ignores non-zoom chords", () => {
    renderZoomProbe();

    const zoomIn = dispatchKeyboardEvent({ key: "=", code: "Equal", ctrlKey: true });
    expect(zoomIn.defaultPrevented).toBe(true);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.10");

    const ignoredAltChord = dispatchKeyboardEvent({ key: "+", code: "Equal", ctrlKey: true, altKey: true });
    expect(ignoredAltChord.defaultPrevented).toBe(false);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.10");

    const zoomOut = dispatchKeyboardEvent({ key: "-", code: "Minus", metaKey: true });
    expect(zoomOut.defaultPrevented).toBe(true);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.10");

    const reset = dispatchKeyboardEvent({ key: "0", code: "Digit0", ctrlKey: true });
    expect(reset.defaultPrevented).toBe(true);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");
  });

  it("uses ctrl/meta wheel only inside the app zoom region", () => {
    renderZoomProbe();

    const region = screen.getByTestId("zoom-region");
    const outside = screen.getByTestId("outside-region");

    const ignoredPlainWheel = dispatchWheelEvent(region, { deltaY: -100 });
    expect(ignoredPlainWheel.defaultPrevented).toBe(false);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");

    const outsideCtrlWheel = dispatchWheelEvent(outside, { deltaY: -100, ctrlKey: true });
    expect(outsideCtrlWheel.defaultPrevented).toBe(true);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.00");

    const smallTrackpadZoom = dispatchWheelEvent(region, { deltaY: -20, ctrlKey: true });
    expect(smallTrackpadZoom.defaultPrevented).toBe(true);
    expect(screen.getByTestId("zoom")).toHaveTextContent("1.05");

    dispatchWheelEvent(region, { deltaY: 100, metaKey: true });
    expect(screen.getByTestId("zoom")).toHaveTextContent("0.95");
  });
});
