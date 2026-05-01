// Electron main process. Loads the Vite dev server in development
// (CORTEX_GUI_DEV=1) and the built `dist/index.html` otherwise. The
// renderer talks to cortex-api over HTTP — there is no IPC bridge
// for application data; the preload script only exposes a tiny
// surface for desktop integrations (open-external-url, etc.) that
// the renderer can't do on its own.

import { app, BrowserWindow, ipcMain, Menu, safeStorage, shell } from "electron";
import * as fs from "node:fs/promises";
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
    // Mirror Vectorizer's custom-titlebar setup: drop the OS
    // chrome entirely so the renderer can paint its own header
    // (drag region + min/max/close buttons) and stay visually
    // consistent across HiveLLM tools.
    frame: false,
    titleBarStyle: "hidden",
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

// Window-control IPC. The renderer paints its own min / max / close
// buttons (no native frame) and asks the main process to drive the
// underlying BrowserWindow via these channels. Same wire shape
// Vectorizer uses so the bridge type stays portable across tools.
ipcMain.on("window-minimize", (event) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  win?.minimize();
});

ipcMain.on("window-maximize", (event) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  if (!win) return;
  if (win.isMaximized()) {
    win.unmaximize();
  } else {
    win.maximize();
  }
});

ipcMain.on("window-close", (event) => {
  const win = BrowserWindow.fromWebContents(event.sender);
  win?.close();
});

// ---------------------------------------------------------------------------
// Phase3 §2 — multi-connection persistence + safeStorage-backed secret store.
//
// `connections.json` lives under `app.getPath("userData")`. The renderer is
// the only writer; the main process serialises it byte-for-byte. We never
// merge — the document the renderer hands us replaces the previous one
// atomically.
//
// Bearer / basic credentials live in `secrets.json` next to it, encrypted
// via Electron's `safeStorage` API. On OSes where encryption is
// unavailable (some Linux desktops without a keyring), we throw so the
// renderer falls back to its plaintext localStorage path; we never write
// unencrypted secrets to disk from the main process.
// ---------------------------------------------------------------------------

const CONNECTIONS_FILE_NAME = "connections.json";
const SECRETS_FILE_NAME = "secrets.json";

function userDataPath(): string {
  return app.getPath("userData");
}

async function readJsonFile<T>(file: string): Promise<T | null> {
  try {
    const raw = await fs.readFile(file, "utf-8");
    return JSON.parse(raw) as T;
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    if (code === "ENOENT") return null;
    throw err;
  }
}

async function writeJsonFileAtomic(file: string, value: unknown): Promise<void> {
  const tmp = `${file}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(value, null, 2), { mode: 0o600 });
  await fs.rename(tmp, file);
}

ipcMain.handle("connections:read", async () => {
  const file = path.join(userDataPath(), CONNECTIONS_FILE_NAME);
  return readJsonFile(file);
});

ipcMain.handle("connections:write", async (_event, state: unknown) => {
  const file = path.join(userDataPath(), CONNECTIONS_FILE_NAME);
  await writeJsonFileAtomic(file, state);
});

type SecretsBlob = Record<string, string>; // base64-encoded ciphertext per key.

async function loadSecrets(): Promise<SecretsBlob> {
  const file = path.join(userDataPath(), SECRETS_FILE_NAME);
  const blob = await readJsonFile<SecretsBlob>(file);
  return blob ?? {};
}

async function persistSecrets(blob: SecretsBlob): Promise<void> {
  const file = path.join(userDataPath(), SECRETS_FILE_NAME);
  await writeJsonFileAtomic(file, blob);
}

ipcMain.handle("secret:read", async (_event, key: unknown): Promise<string | null> => {
  if (typeof key !== "string" || !key) return null;
  if (!safeStorage.isEncryptionAvailable()) return null;
  const blob = await loadSecrets();
  const cipher = blob[key];
  if (!cipher) return null;
  try {
    return safeStorage.decryptString(Buffer.from(cipher, "base64"));
  } catch {
    return null;
  }
});

ipcMain.handle(
  "secret:write",
  async (_event, payload: unknown): Promise<void> => {
    if (!safeStorage.isEncryptionAvailable()) {
      throw new Error("safeStorage encryption unavailable on this platform");
    }
    if (
      typeof payload !== "object" ||
      payload === null ||
      typeof (payload as { key?: unknown }).key !== "string" ||
      typeof (payload as { value?: unknown }).value !== "string"
    ) {
      throw new Error("secret:write payload must be { key: string, value: string }");
    }
    const { key, value } = payload as { key: string; value: string };
    const blob = await loadSecrets();
    blob[key] = safeStorage.encryptString(value).toString("base64");
    await persistSecrets(blob);
  },
);

ipcMain.handle("secret:remove", async (_event, key: unknown): Promise<void> => {
  if (typeof key !== "string" || !key) return;
  const blob = await loadSecrets();
  if (key in blob) {
    delete blob[key];
    await persistSecrets(blob);
  }
});

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
