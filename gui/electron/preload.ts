// Preload script. Runs in an isolated context with access to a
// limited slice of Node APIs; the main world (the renderer) sees
// only what we explicitly bridge here. The MVP surface is tiny —
// the renderer fetches everything it needs directly from
// cortex-api over HTTP.

import { contextBridge, ipcRenderer } from "electron";

// `window.cortex` is the surface available to the renderer. Keep it
// minimal so we don't grow an IPC API ahead of need.
contextBridge.exposeInMainWorld("cortex", {
  /// Build identifier — surfaced in the header for support / bug
  /// reports.
  buildId: process.env.npm_package_version ?? "dev",
  /// Whether the app booted in dev mode (drives the "live reload"
  /// indicator in the header).
  isDev: process.env.CORTEX_GUI_DEV === "1",
  /// Window-control bridge. Custom titlebar drops the OS chrome,
  /// so the renderer drives min / max / close through these
  /// channels. Same shape Vectorizer's GUI uses.
  windowMinimize: () => ipcRenderer.send("window-minimize"),
  windowMaximize: () => ipcRenderer.send("window-maximize"),
  windowClose: () => ipcRenderer.send("window-close"),
});
