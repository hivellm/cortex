# Cortex GUI

Desktop dashboard for Cortex. Electron 33 + Vite 5 + React 18 + TypeScript.
Renderer talks to `cortex-api` over HTTP — no IPC bridge for application
data, only the tiny `window.cortex` surface for desktop integrations
(open-external, build id).

## Layout

```
gui/
├── package.json            # pnpm workspace root for the GUI
├── tsconfig.json           # renderer TS config
├── tsconfig.electron.json  # main + preload TS config
├── vite.config.ts          # renderer build + dev proxy to cortex-api
├── index.html              # Vite renderer entry
├── electron/
│   ├── main.ts             # Electron main process
│   └── preload.ts          # contextBridge surface (window.cortex)
├── src/
│   ├── main.tsx            # React entry + TanStack Query provider
│   ├── App.tsx             # top-level shell
│   ├── styles.css          # ported verbatim from gui/assets/styles.css
│   ├── lib/
│   │   ├── api.ts          # fetchers for /v1/dashboard/*
│   │   └── format.ts       # fmtNum / sevTone / kindLabel
│   ├── atoms/
│   │   ├── Icon.tsx        # SVG icon set, ported from atoms.jsx
│   │   ├── Sparkline.tsx
│   │   └── Tag.tsx
│   ├── shell/
│   │   ├── Header.tsx
│   │   └── Sidebar.tsx
│   └── views/
│       └── Timeline.tsx    # MVP — wires to /v1/dashboard/timeline/recent
└── assets/                 # design reference (the original prototype, untouched)
```

The `assets/` tree at sibling-level (`gui/assets/`) is the design source —
read-only reference. Production code under `src/` ports the visual
language but does not import from `assets/`.

## Develop

```bash
# Backend running on 127.0.0.1:15011 with archive seed:
CORTEX_ARCHIVE_ROOT=~/.cortex/archive cargo run -p cortex-api

# In gui/:
pnpm install
pnpm dev               # vite + electron, reload on file change
```

Renderer dev server runs on `http://127.0.0.1:5173`. Electron auto-loads
it once `wait-on tcp:5173` clears. The Vite dev proxy forwards `/v1/*`
to `http://127.0.0.1:15011`, so fetch() calls in the renderer resolve
without CORS.

## Build

```bash
pnpm build             # vite build → dist/, tsc → dist-electron/
pnpm start             # run the packaged Electron app
```

## Status

Spec 16 §0 MVP — Timeline view wired to live `cortex-api` data.
The remaining six views (Memory, Decisions, Laws, Analysis, Tools,
Graph) ship under `phase2_dashboard` §5; until then they render a
"coming next" panel that points back at the design reference.
