import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Phase3 — minimal vitest config so `pnpm --filter ./gui test`
// runs the Inspector + filter coverage. Browser-shaped tests use
// jsdom; node-only files (api.ts mappers) opt in per-file via
// `// @vitest-environment node`.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    setupFiles: ["./vitest.setup.ts"],
  },
});
