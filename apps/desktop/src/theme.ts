export const darkTheme = {
  "--background": "224, 5%, 8%",        // Deep dark background reminiscent of Spacedrive
  "--foreground": "220, 4%, 98%",       // Almost white text
  "--card": "224, 6%, 12%",             // Slightly lighter than bg for depth
  "--card-foreground": "220, 4%, 95%",
  "--primary": "217, 91%, 60%",         // Spacedrive-like vibrant blue
  "--primary-foreground": "0, 0%, 100%",
  "--secondary": "215, 15%, 18%",       // Muted secondary elements
  "--secondary-foreground": "220, 4%, 85%",
  "--muted": "215, 10%, 25%",           // More subtle muted color
  "--muted-foreground": "215, 10%, 65%",
  "--accent": "217, 91%, 60%",          // Same as primary for consistency
  "--accent-foreground": "0, 0%, 100%",
  "--destructive": "0, 70%, 50%",       // Red
  "--destructive-foreground": "0, 0%, 100%",
  "--border": "215, 10%, 20%",          // Subtle border
  "--input": "224, 6%, 12%",            // Input field matching card
  "--ring": "217, 91%, 60%",            // Focus ring matching primary
  "--radius": "0.5rem",                 // More modern rounded corners
};

/**
 * Light theme inspired by Spacedrive
 */
export const lightTheme = {
  "--background": "0, 0%, 98%",         // Clean light background
  "--foreground": "224, 5%, 8%",        // Dark text for contrast
  "--card": "0, 0%, 96%",               // Slightly off-white cards
  "--card-foreground": "224, 5%, 8%",
  "--primary": "217, 91%, 60%",         // Spacedrive-like vibrant blue
  "--primary-foreground": "0, 0%, 100%",
  "--secondary": "210, 20%, 90%",       // Light secondary background
  "--secondary-foreground": "224, 5%, 8%",
  "--muted": "210, 15%, 92%",           // Light muted color
  "--muted-foreground": "215, 10%, 40%",
  "--accent": "217, 91%, 60%",          // Same as primary
  "--accent-foreground": "0, 0%, 100%",
  "--destructive": "0, 70%, 50%",       // Red
  "--destructive-foreground": "0, 0%, 100%",
  "--border": "210, 15%, 85%",          // Soft borders
  "--input": "0, 0%, 98%",              // Input fields
  "--ring": "217, 91%, 60%",            // Focus ring
  "--radius": "0.5rem",                 // More modern rounded corners
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
