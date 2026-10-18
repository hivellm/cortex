# Cortex GUI

Shared dashboard renderer for Cortex: React 19, Vite 7 and TypeScript, with
retained Electron 39 desktop support. The standalone `cortex serve` executable
embeds this renderer and serves local APIs. Legacy Electron mode still talks to
`cortex-api` over HTTP; its `window.cortex` bridge covers desktop integrations
(open-external, build id), not application data.

## Standalone build and runtime

From the repository root, with the repository Rust toolchain, Node.js 22.12+
and pnpm 10:

```bash
ELECTRON_SKIP_BINARY_DOWNLOAD=1 pnpm --dir gui install --frozen-lockfile
cargo build --release --locked -p cortex-local --bin cortex
target/release/cortex --repository Cortex serve
```

In PowerShell, set `$env:ELECTRON_SKIP_BINARY_DOWNLOAD = "1"` before `pnpm`;
use `target/release/cortex.exe`. Cargo invokes `scripts/build-standalone.mjs`,
builds into its own `OUT_DIR`, and embeds the generated assets. No generated
`dist/` files need committing. The installed executable needs no Node.js,
Electron binary, frontend server or Docker.

Standalone mode shares `HeaderFrame`, `SidebarFrame`, `Tweaks`, atoms and
`styles.css` with the existing dashboard. `src/local/` supplies local API/state
integration and view bodies; mobile navigation uses an icon rail. The build
injects the standalone runtime marker. Legacy routes and services are not
requested in this mode.

Open the token-bearing URL printed by `serve`. Bootstrap removes its fragment
before React mounts and retains the per-process token in tab session storage.
The server checks Host, Origin and bearer authorization on local API requests.
Repository selection is exact and case-sensitive. Inventory reports complete
current-record and link counts, while record pages use bounded, filter-bound
cursors. Timeline, memory, decisions, graph, classifications, health, audit and
feedback render local store data. Capture, revisions and source inspection
remain available. Queueing classification does not call Jev; feedback ranking
starts disabled.

Run renderer checks from `gui/`:

```bash
pnpm typecheck
pnpm lint
pnpm test
```

ESLint applies JavaScript and TypeScript recommended checks, browser/Node
scopes and React Hooks rules. Warnings fail the gate; type checking remains a
separate compiler check.

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

## Legacy Electron development

```bash
# Backend running on 127.0.0.1:17000 with archive seed:
CORTEX_ARCHIVE_ROOT=~/.cortex/archive cargo run -p cortex-api

# In gui/:
pnpm install --frozen-lockfile
pnpm dev               # vite + electron, reload on file change
```

Renderer dev server runs on `http://127.0.0.1:5173`. Electron auto-loads
it once `wait-on tcp:5173` clears. The Vite dev proxy forwards `/v1/*`
to `http://127.0.0.1:17000`, so fetch() calls in the renderer resolve
without CORS.

## Legacy Electron build

```bash
pnpm build             # vite build → dist/, tsc → dist-electron/
pnpm start             # run the packaged Electron app
```

## Legacy service views

The following describes the retained service-backed Electron mode, not the
standalone local routes or their acceptance results:

| View              | Endpoint                            | Notes                                                                 |
|-------------------|-------------------------------------|-----------------------------------------------------------------------|
| Live timeline     | `/v1/dashboard/timeline/recent` (+ `/overview` + `/sessions`) | 6-tile stats grid (Events/min · Pre-thinking P95 · Repos active · Tool calls/Turns · Classifier spend·24h · Violations·7d) with `Sparkline` trends from `overview.series.{events_per_min, pre_thinking_p95_ms, classifier_cost_usd_today, violations_7d_daily}`. P95 reads `extras.duration_ms` from the lane (tool/agent calls today; spec-12 widens to turns) and renders gaps for empty buckets. Classifier spend renders `—` while `classifier_cost_unavailable_until_spec05: true` — flips to a real ribbon once spec-05 stamps cost. `Pause stream` / `Resume` toggles `useQuery`'s `refetchInterval`; footer pill reads `● connected` / `○ paused`. New rows since the last fetch flash with the `is-new` keyframe for ~700 ms; first fetch primes the seen-ids set without flashing every row. Row click opens slide-in inspector with the full envelope. |
| Memory            | `/v1/dashboard/memory`              | Faceted by canonical kind (`turn / tool_call / agent_call / decision / analysis`). |
| Decisions         | `/v1/dashboard/decisions`           | "Decision register" — `Show superseded` toggle, `supersedes` / `superseded → …` inline tags, `is-superseded` opacity, optional `chain` renders the horizontal supersession chain (graceful when absent). Empty until `kind=decision` envelopes flow. |
| Laws              | `/v1/dashboard/laws` + `/violations` + `/trust` | "Law dashboard" — 4-tile stats (Blocking laws · Observational · False-block rate · Trust score range), Active-laws card (header row · `SeverityBar` atom · `block`/`observe` action cell · sorted by `violations_7d` desc), Trust score `model × repo` heatmap with `oklch` ramp; empty-state copy switches on `trust.source === "stub_until_spec14"` so phase2g's stub envelope reads as honest missing data, and recent-violations list. |
| Analysis          | `/v1/dashboard/analyses`            | Stats grid + cards. Empty until spec-15 emits `kind=analysis`.        |
| Tool analytics    | `/v1/dashboard/tools/stats`         | `avg_ms` and `err_rate` are 0 until spec-12 derivation lands. The response also carries a 7×24 `heatmap` of weekday × hour-of-day tool-call counts (UTC) — phase2g §2 — that the Tools view renders with the design's `oklch` intensity formula. |
| Graph explorer    | `/v1/dashboard/graph`               | Synthetic Session→Turn→ToolCall layout from the seeded archive.       |

Sidebar lists captured sessions (`/v1/dashboard/sessions`) and supports
session/repo/kind filters that flow through every list view.

Header pill and sidebar footer reflect `/v1/status` — green when the
daemon answers, grey when it does not.

### Tweaks

A slide-in panel triggered by the header gear icon, exposing the operator-tunable surface backed by `localStorage` under key `cortex.tweaks`. Mirrors the `.inspector` chrome (right-anchored slide, ESC + outside-click close). Source: [`shell/Tweaks.tsx`](src/shell/Tweaks.tsx) + [`lib/useTweaks.tsx`](src/lib/useTweaks.tsx).

| Group   | Control                                                               | Driven CSS                                                  |
|---------|-----------------------------------------------------------------------|-------------------------------------------------------------|
| Theme   | Dark / Light radio                                                    | `document.documentElement.dataset.theme`                    |
| Color   | 5 preset chips (Amber 75 · Green 155 · Blue 230 · Purple 290 · Red 25) + 20°–320° hue slider | `--accent-h` (consumed by `--accent` and every `--accent-*` token via `oklch(... var(--accent-h))`) |
| Layout  | Collapse-sidebar checkbox + 1–10 density slider                       | `--header-h: calc(52 - (10 - density) × 0.8)px`             |
| About   | Read-only `service / version / pid / uptime` from `/v1/status`        | —                                                           |

A "Reset to defaults" button at the bottom of the drawer flips the store back to `{ theme: dark, accentHue: 75, density: 7, sidebarCollapsed: false }`. Closing the drawer doesn't revert anything — the tweak is committed on every change.

### Inspectors

Two slide-in inspectors share the chrome (`.inspector` + `.inspector-backdrop` in `styles.css`); ESC and outside-click close both.

| Inspector       | Source                | Sections                                                                                                  |
|-----------------|-----------------------|-----------------------------------------------------------------------------------------------------------|
| Event inspector | `views/Timeline.tsx`  | Detail · Envelope (id / kind / session / repo / model / at) · **Payload (redacted)** (full JSON via `<pre class="code-block">`) · **Linked** (DEC- / ANL- ids found in the title surface as clickable cards; "no linked decisions or analyses" otherwise — no fabricated links). |
| Law inspector   | `views/Laws.tsx`      | Head (severity icon + id + severity / blocking tag) · Definition (synthesized YAML frontmatter from the row) · 7-day stats (`applies / violations / rate / action`) · Recent violations (subset of `/v1/dashboard/violations` filtered by `law_id`).                          |

### Atoms

Reusable visual primitives under `src/atoms/`:

| Atom         | Where it lives                  | Used by                                                                                |
|--------------|---------------------------------|----------------------------------------------------------------------------------------|
| `Icon`       | `atoms/Icon.tsx`                | every view (sidebar nav, view actions, timeline kind glyphs, inspector chrome)         |
| `Sparkline`  | `atoms/Sparkline.tsx`           | Sidebar workspace pulse (last-20-minutes events-per-min); Timeline stats grid          |
| `Bars`       | `atoms/Bars.tsx`                | Tool analytics bar chart                                                               |
| `SeverityBar`| `atoms/SeverityBar.tsx`         | Laws table + Law inspector severity column (3-segment info / notable / critical ramp) |
| `Tag`        | `atoms/Tag.tsx`                 | every view — `default / ok / warn / critical / info / accent / solid` tone variants    |

The atoms map 1:1 to `gui/assets/atoms.jsx` (the design source). Production code under `src/atoms/` ports the visual language but does not import from `gui/assets/`.

### Not yet wired

- **SSE / WebSocket** for the timeline (still polls every 5s).
- **Decision supersession chain** — the renderer (`SupersedeChain`)
  ships and renders gracefully when the backend includes
  `chain: [{id, title, state}]` on a `DecisionRow`. Phase2h adds the
  field; until then, decisions render without a chain.
- **Trust matrix** — the renderer (`TrustGrid`) is wired against
  `/v1/dashboard/trust` with the design's `oklch` ramp, but the
  endpoint stays empty until spec-14 derivation lands.
