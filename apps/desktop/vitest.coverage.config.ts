import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    include: [
      "src/**/*.test.ts",
      "src/**/*.test.tsx",
      "src/integration/**/*.test.ts",
      "src/integration/**/*.test.tsx",
    ],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary", "lcov"],
      reportsDirectory: "./coverage/frontend",
      exclude: [
        "src/assets/**",
        "src/test/**",
        "src/components/ui/**",
        "src/main.tsx",
        "src/App.tsx",
        "src/vite-env.d.ts",
        "**/*.svg",
        "**/*.png",
        "**/*.apng",
        "**/*.icns",
        "**/*.ico",
      ],
    },
  },
});
