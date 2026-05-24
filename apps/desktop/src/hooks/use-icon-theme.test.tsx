import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_ICON_THEME,
  ICON_THEME_LABELS,
  ICON_THEME_OPTIONS,
  IconThemeProvider,
  useIconTheme,
} from "./use-icon-theme";

function ThemeProbe() {
  const { theme, setTheme } = useIconTheme();
  return (
    <div>
      <span data-testid="theme">{theme}</span>
      <button type="button" onClick={() => setTheme("classic")}>Classic</button>
      <button type="button" onClick={() => setTheme("modern")}>Modern</button>
    </div>
  );
}

describe("useIconTheme", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("exports stable theme options and labels", () => {
    expect(DEFAULT_ICON_THEME).toBe("vivid");
    expect(ICON_THEME_OPTIONS).toEqual(["classic", "modern", "vivid", "square"]);
    expect(ICON_THEME_LABELS).toMatchObject({
      classic: "Classic",
      modern: "Modern",
      vivid: "Vivid",
      square: "Square",
    });
  });

  it("uses the default theme and persists updates", () => {
    render(
      <IconThemeProvider>
        <ThemeProbe />
      </IconThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("vivid");
    fireEvent.click(screen.getByRole("button", { name: "Classic" }));

    expect(screen.getByTestId("theme")).toHaveTextContent("classic");
    expect(window.localStorage.getItem("infimount.iconTheme")).toBe("classic");
  });

  it("loads a valid stored theme and ignores invalid stored values", async () => {
    window.localStorage.setItem("infimount.iconTheme", "modern");
    const { unmount } = render(
      <IconThemeProvider>
        <ThemeProbe />
      </IconThemeProvider>,
    );

    await waitFor(() => expect(screen.getByTestId("theme")).toHaveTextContent("modern"));
    unmount();

    window.localStorage.setItem("infimount.iconTheme", "unknown");
    render(
      <IconThemeProvider>
        <ThemeProbe />
      </IconThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("vivid");
  });
});
