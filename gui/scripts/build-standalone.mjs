import { build } from "vite";
import { resolve, join, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

const gui = fileURLToPath(new URL("..", import.meta.url));
const output = process.argv[2];
if (!output || !isAbsolute(output) || !process.env.OUT_DIR ||
    resolve(output) !== join(resolve(process.env.OUT_DIR), "dashboard")) {
  throw new Error("Standalone renderer output must be OUT_DIR/dashboard from Cargo.");
}

await build({
  configFile: resolve(gui, "vite.config.ts"),
  root: gui,
  base: "/",
  plugins: [{
    name: "cortex-standalone-runtime",
    transformIndexHtml(html) {
      return {
        html: html.replace(/<meta\s+http-equiv="Content-Security-Policy"[^>]*>/i, ""),
        tags: [{ tag: "meta", attrs: { name: "cortex-runtime", content: "standalone" }, injectTo: "head-prepend" }],
      };
    },
  }],
  build: { outDir: output, emptyOutDir: true, sourcemap: false },
});
