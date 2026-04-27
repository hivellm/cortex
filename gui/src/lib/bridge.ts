/**
 * Renderer-side typings for the `window.cortex` surface exposed by
 * `electron/preload.ts`. The MVP surface is intentionally tiny — we
 * only mirror what the preload bridge actually publishes today, so
 * the renderer cannot assume capabilities the main process never
 * granted.
 */

export type CortexBridge = {
  /** Build identifier — surfaced in the header. */
  buildId: string;
  /** True when launched with `CORTEX_GUI_DEV=1`. */
  isDev: boolean;
};

declare global {
  interface Window {
    cortex?: CortexBridge;
  }
}

export const bridge: CortexBridge = (typeof window !== "undefined" && window.cortex) || {
  buildId: "dev",
  isDev: false,
};
