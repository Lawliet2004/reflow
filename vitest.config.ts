import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

/**
 * Kept separate from `vite.config.ts` so the Tauri dev-server settings
 * (fixed port, strictPort, HMR host) never apply to a test run.
 */
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    coverage: {
      reporter: ["text", "lcov"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**"],
    },
  },
});
