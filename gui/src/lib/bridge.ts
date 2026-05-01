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
  /** Minimize the host BrowserWindow (custom titlebar). */
  windowMinimize?: () => void;
  /** Toggle maximize/unmaximize (custom titlebar). */
  windowMaximize?: () => void;
  /** Close the host BrowserWindow (custom titlebar). */
  windowClose?: () => void;
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

/// `true` when the renderer is running inside Electron (the preload
/// bridge wired the window-control channels). The browser preview
/// at `127.0.0.1:5173` renders the same app without these, so the
/// custom titlebar buttons stay hidden there.
export const isElectron: boolean =
  typeof window !== "undefined" &&
  typeof window.cortex?.windowMinimize === "function";
