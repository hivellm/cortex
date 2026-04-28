import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

// Vite config for the renderer process. Built output goes to
// `dist/` which the Electron main process loads in production. Dev
// mode runs against `http://localhost:5173/` which `electron/main.ts`
// detects via the `CORTEX_GUI_DEV=1` env var.
export default defineConfig({
  plugins: [react()],
  root: resolve(__dirname),
  base: "./",
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    proxy: {
      // Forward `/v1/*` API calls to cortex-api during dev so the
      // browser's same-origin policy doesn't bite on fetch().
      "/v1": {
        target: "http://127.0.0.1:15000",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
  },
});
