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
  /// Phase3 §2 — multi-connection persistence bridge. Reads /
  /// writes `connections.json` under `app.getPath("userData")`.
  /// The renderer never touches `fs` directly; main process owns
  /// the JSON document, the bridge funnels it across the
  /// context-isolation boundary.
  connections: {
    read: (): Promise<unknown> => ipcRenderer.invoke("connections:read"),
    write: (state: unknown): Promise<void> =>
      ipcRenderer.invoke("connections:write", state),
  },
  /// Phase3 §2.2 — `safeStorage`-backed bearer-token storage. Keys
  /// scoped per-connection via `bearer:<connectionId>` /
  /// `basic:<connectionId>` so removing a connection only clears
  /// its own credential. The OS keychain handles the heavy
  /// lifting; on platforms where `safeStorage.isEncryptionAvailable()`
  /// returns false the main process raises and the renderer falls
  /// back to the localStorage path.
  secret: {
    read: (key: string): Promise<string | null> =>
      ipcRenderer.invoke("secret:read", key),
    write: (key: string, value: string): Promise<void> =>
      ipcRenderer.invoke("secret:write", { key, value }),
    remove: (key: string): Promise<void> =>
      ipcRenderer.invoke("secret:remove", key),
  },
});
