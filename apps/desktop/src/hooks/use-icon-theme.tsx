import * as React from "react";

export type IconTheme = "classic" | "modern" | "vivid" | "square";

export const DEFAULT_ICON_THEME: IconTheme = "vivid";
export const ICON_THEME_OPTIONS: IconTheme[] = ["classic", "modern", "vivid", "square"];
export const ICON_THEME_LABELS: Record<IconTheme, string> = {
  classic: "Classic",
  modern: "Modern",
  vivid: "Vivid",
  square: "Square",
};

const STORAGE_KEY = "openhsb.iconTheme";

type IconThemeContextValue = {
  theme: IconTheme;
  setTheme: (theme: IconTheme) => void;
};

const IconThemeContext = React.createContext<IconThemeContextValue>({
  theme: DEFAULT_ICON_THEME,
  setTheme: () => {},
});

export const IconThemeProvider = ({ children }: { children: React.ReactNode }) => {
  const [theme, setThemeState] = React.useState<IconTheme>(DEFAULT_ICON_THEME);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const stored = window.localStorage.getItem(STORAGE_KEY) as IconTheme | null;
    if (stored && ICON_THEME_OPTIONS.includes(stored)) {
      setThemeState(stored);
    }
  }, []);

  const setTheme = React.useCallback((next: IconTheme) => {
    setThemeState(next);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(STORAGE_KEY, next);
    }
  }, []);

  const value = React.useMemo(() => ({ theme, setTheme }), [theme, setTheme]);

  return <IconThemeContext.Provider value={value}>{children}</IconThemeContext.Provider>;
};

export const useIconTheme = () => React.useContext(IconThemeContext);
