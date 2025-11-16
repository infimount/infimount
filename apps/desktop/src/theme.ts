export const darkTheme = {
  "--background": "222, 28%, 10%",      // Deep blue-gray
  "--foreground": "210, 40%, 98%",      // Light text
  "--card": "222, 25%, 15%",            // Slightly lighter than bg
  "--card-foreground": "235, 35%, 92%",
  "--primary": "258, 90%, 60%",         // Purple
  "--primary-foreground": "0, 0%, 100%",
  "--secondary": "200, 75%, 55%",       // Cyan-blue
  "--secondary-foreground": "0, 0%, 100%",
  "--muted": "235, 15%, 40%",           // Medium gray
  "--muted-foreground": "235, 15%, 70%",
  "--accent": "280, 85%, 55%",          // Vibrant purple
  "--accent-foreground": "0, 0%, 100%",
  "--destructive": "0, 84%, 60%",       // Red
  "--destructive-foreground": "0, 0%, 100%",
  "--border": "235, 15%, 25%",          // Subtle border
  "--input": "222, 25%, 20%",           // Input field
  "--ring": "258, 90%, 60%",            // Focus ring
};

/**
 * Light theme
 */
export const lightTheme = {
  "--background": "210, 40%, 96%",      // Light bluish background
  "--foreground": "222, 22%, 25%",      // Dark text
  "--card": "0, 0%, 100%",              // White cards
  "--card-foreground": "222, 22%, 25%",
  "--primary": "213, 90%, 58%",         // Bright blue
  "--primary-foreground": "0, 0%, 100%",
  "--secondary": "210, 40%, 96%",       // Muted blue background
  "--secondary-foreground": "222, 22%, 25%",
  "--muted": "210, 24%, 90%",           // Light gray-blue
  "--muted-foreground": "215, 16%, 47%",
  "--accent": "213, 90%, 58%",          // Same as primary
  "--accent-foreground": "0, 0%, 100%",
  "--destructive": "0, 84%, 60%",       // Red
  "--destructive-foreground": "0, 0%, 100%",
  "--border": "210, 24%, 88%",          // Soft borders
  "--input": "0, 0%, 100%",             // White inputs
  "--ring": "213, 90%, 58%",            // Focus ring
};

export type Theme = "dark" | "light";

/**
 * Get theme values based on theme name
 */
export function getThemeColors(theme: Theme) {
  return theme === "dark" ? darkTheme : lightTheme;
}

/**
 * Apply theme to document
 */
export function applyTheme(theme: Theme) {
  const colors = getThemeColors(theme);
  const root = document.documentElement;

  Object.entries(colors).forEach(([key, value]) => {
    root.style.setProperty(key, value);
  });

  // Add/remove dark class for Tailwind
  if (theme === "dark") {
    root.classList.add("dark");
  } else {
    root.classList.remove("dark");
  }
}
