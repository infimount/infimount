import { create } from "zustand";
import { persist } from "zustand/middleware";
import { type Theme, applyTheme } from "../theme";

interface ThemeStore {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  toggleTheme: () => void;
}

export const useThemeStore = create<ThemeStore>()(
  persist(
    (set) => ({
      theme: "light",
      setTheme: (theme: Theme) => {
        applyTheme(theme);
        set({ theme });
      },
      toggleTheme: () => {
        set((state) => {
          const newTheme = state.theme === "dark" ? "light" : "dark";
          applyTheme(newTheme);
          return { theme: newTheme };
        });
      },
    }),
    {
      name: "theme-store",
      onRehydrateStorage: () => (state) => {
        // Apply theme on hydration
        if (state) {
          applyTheme(state.theme);
        }
      },
    }
  )
);
