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

All seven views wired to live `cortex-api` data:

| View              | Endpoint                            | Notes                                                                 |
|-------------------|-------------------------------------|-----------------------------------------------------------------------|
| Live timeline     | `/v1/dashboard/timeline/recent`     | Polls every 5s; row click opens slide-in inspector with envelope.     |
| Memory            | `/v1/dashboard/memory`              | Faceted by canonical kind (`turn / tool_call / agent_call / decision / analysis`). |
| Decisions         | `/v1/dashboard/decisions`           | Stats grid + list. Empty until `kind=decision` envelopes flow.        |
| Laws              | `/v1/dashboard/laws` + `/violations`| Catalogue empty until spec-13 ships; violations list lives.           |
| Analysis          | `/v1/dashboard/analyses`            | Stats grid + cards. Empty until spec-15 emits `kind=analysis`.        |
| Tool analytics    | `/v1/dashboard/tools/stats`         | `avg_ms` and `err_rate` are 0 until spec-12 derivation lands.         |
| Graph explorer    | `/v1/dashboard/graph`               | Synthetic Session→Turn→ToolCall layout from the seeded archive.       |

Sidebar lists captured sessions (`/v1/dashboard/sessions`) and supports
session/repo/kind filters that flow through every list view.

Header pill and sidebar footer reflect `/v1/status` — green when the
daemon answers, grey when it does not.

### Not yet wired

- **SSE / WebSocket** for the timeline (still polls every 5s).
- **Tweaks panel** (theme/accent/density) from the design reference.
- **Decision supersession chain** rendering.
- **Trust matrix** and tool-call **heatmap** — both need data shapes
  the backend doesn't emit yet.
