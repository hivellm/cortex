// Electron main process. Loads the Vite dev server in development
// (CORTEX_GUI_DEV=1) and the built `dist/index.html` otherwise. The
// renderer talks to cortex-api over HTTP — there is no IPC bridge
// for application data; the preload script only exposes a tiny
// surface for desktop integrations (open-external-url, etc.) that
// the renderer can't do on its own.

import { app, BrowserWindow, Menu, shell } from "electron";
import * as path from "node:path";

const isDev = process.env.CORTEX_GUI_DEV === "1";
const devUrl = "http://127.0.0.1:5173";

// Drop the default application menu (File / Edit / View / Window /
// Help). The dashboard owns its own header chrome inside the
// renderer and the OS menu bar would only steal vertical space.
Menu.setApplicationMenu(null);

async function createWindow(): Promise<void> {
  const win = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1024,
    minHeight: 640,
    backgroundColor: "#0d1117",
    show: false,
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  // Belt-and-suspenders: even with autoHideMenuBar the user can flash
  // the menu via Alt; remove the menu reference from this window so
  // there's nothing to flash.
  win.setMenu(null);

  // Render external links in the OS browser — never inside the app.
  win.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: "deny" };
  });

  win.once("ready-to-show", () => {
    win.show();
    if (isDev) win.webContents.openDevTools({ mode: "detach" });
  });

  if (isDev) {
    await loadWithRetry(win, devUrl);
  } else {
    // Built renderer is colocated next to dist-electron/.
    await win.loadFile(path.join(__dirname, "..", "dist", "index.html"));
  }
}

async function loadWithRetry(
  win: BrowserWindow,
  url: string,
  attempts = 20,
  delayMs = 250,
): Promise<void> {
  // Vite + Electron race on first boot — wait-on tcp:5173 lifts the
  // socket-listening latch but the HTTP handler may still be a few
  // ms behind. Retry the loadURL with a short backoff so the user
  // never sees a "ERR_CONNECTION_REFUSED" blank window.
  let lastErr: unknown;
  for (let i = 0; i < attempts; i++) {
    try {
      await win.loadURL(url);
      return;
    } catch (e) {
      lastErr = e;
      await new Promise((r) => setTimeout(r, delayMs));
    }
  }
  console.error("electron: failed to load", url, "after", attempts, "attempts:", lastErr);
}

app.whenReady().then(async () => {
  await createWindow();
  app.on("activate", async () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      await createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  // Stay alive on macOS until the user quits explicitly (standard
  // platform convention); on Windows / Linux quit when the last
  // window closes.
  if (process.platform !== "darwin") app.quit();
});
