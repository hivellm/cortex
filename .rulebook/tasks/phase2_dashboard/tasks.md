## 0. MVP slice — pragmatic single-session cut

Goal: open a browser at `http://127.0.0.1:15011/dashboard/` and see the
Timeline view rendering the user's actual captured events. Stands up the
existing `gui/assets/` prototype served verbatim from `cortex-api`, swapping
its `MOCK` data for live fetchers. The §1–§9 plan below stays as the
durable production target.

- [ ] 0.1 `cortex-api` mounts `/dashboard/*` static-file route serving `gui/assets/` (the prototype already bootstraps via Babel-standalone, no build step needed for the MVP)
- [ ] 0.2 `GET /v1/dashboard/overview` returns counters from the in-memory keyword lane (`events_total`, `repos_indexed`, `kind_breakdown`, `recent_repos`) — JSON shape matches `MOCK.overview` so the prototype's `OverviewView` renders against it without code change
- [ ] 0.3 `GET /v1/dashboard/timeline/recent?limit=N` returns the most-recent N captured envelopes mapped to the `MOCK.events` shape (`id`, `t`, `kind`, `title`, `detail`, `repo`); reuses the archive_loader's lane hits as the source of truth (no SSE in MVP — periodic polling from the SPA covers it)
- [ ] 0.4 `GET /v1/dashboard/memory?q=...` wraps `cortex.query intent=free_search` with a result shape matching `MOCK.memories` (`title`, `excerpt`, `kind`, `repo`, `topics`, `updated`)
- [ ] 0.5 `gui/assets/data.js` learns to fetch live data on boot — `MOCK` becomes the initial state; a small bootstrap script re-fills `MOCK.overview` / `MOCK.events` / `MOCK.memories` from the new endpoints, then a 5-second `setInterval` keeps Timeline fresh; auth-key support is moved to §2 of this document
- [ ] 0.6 Manual smoke: `cortex-api` running with `CORTEX_ARCHIVE_ROOT` set; open `http://127.0.0.1:15011/dashboard/`; Timeline view shows the user's prompts the way `/cortex-query` already returns them; Memory view's free-text search hits the live archive
- [ ] 0.7 Tests: integration test for each new endpoint (overview/timeline-recent/memory) round-tripping the response shape; backend unit tests cover the MOCK-shape contract

## 1. Backend endpoints (cortex-api)
- [ ] 1.1 `cortex-api/src/dashboard/` module with router prefix `/v1/dashboard` and shared response types matching `gui/assets/data.js` shapes
- [ ] 1.2 `GET /v1/dashboard/overview` — today's counters (events emitted by kind, active laws, in-progress analyses, top tools, classifier spend) plus the four sparkline series the prototype renders (`turns`, `tools`, `violations`, `classifier`)
- [ ] 1.3 `GET /v1/dashboard/timeline/stream` — SSE feed of `cortex.events.*` filtered by `?repo`, `?model`, `?severity`; reconnect-friendly with `Last-Event-ID`; events match the prototype's `MOCK.events` shape (`id`, `t`, `kind`, `title`, `detail`, `session`, `model`, `repo`, `duration`)
- [ ] 1.4 `GET /v1/dashboard/memory` — wraps `/v1/query intent=free_search` with facet aggregation; response shape matches `MOCK.memories` (`title`, `excerpt`, `kind`, `repo`, `topics`, `updated`)
- [ ] 1.5 `GET /v1/dashboard/decisions` + `GET /v1/dashboard/decisions/{id}` — Markdown body, supersession `chain` array (per `MOCK.decisions[].chain`), linked Analysis + Turns
- [ ] 1.6 `GET /v1/dashboard/laws` + `GET /v1/dashboard/laws/{id}` + `POST /v1/dashboard/laws` (draft) + `POST /v1/dashboard/laws/{id}/publish`; law shape matches `MOCK.laws` (`id`, `title`, `severity`, `blocked`, `scope`, `applies`, `violations7d`, `rate`, `detector`, `remediation`); violations match `MOCK.violations`
- [ ] 1.7 `GET /v1/dashboard/analyses` + `GET /v1/analysis/{id}/stream`; analysis shape matches `MOCK.analyses` (`id`, `title`, `status`, `panel`, `judge`, `rounds`, `durationS`, `decisionId`, `verdict`)
- [ ] 1.8 `GET /v1/dashboard/tools/stats?since=...` rolls up event-archive Parquet via DuckDB; response carries the `MOCK.toolUsage` shape plus the `tool × hour` heatmap matrix
- [ ] 1.9 `GET /v1/dashboard/graph?session_id=...` returns `nodes` + `edges` per `MOCK.graph` so the SPA's inline SVG renderer (in `view-graph.jsx`) can render without re-fetching
- [ ] 1.10 `GET /v1/dashboard/trust` (stub returning empty list until spec 14 lands) — model × repo grid the `LawsView` already renders
- [ ] 1.11 `POST /v1/dashboard/rum` for client RUM beacons

## 2. Auth + ACL
- [ ] 2.1 `Authorization: Bearer <api_key>` middleware on every `/v1/dashboard/*` route + the SSE stream
- [ ] 2.2 `cortex admin issue-api-key --scope dashboard` sub-command on `cortex-api` binary
- [ ] 2.3 OIDC `onTokenAcquired` hook stubbed for the future
- [ ] 2.4 401 / 429 responses match the rest of `cortex-api` (`reason` body shape)

## 3. SPA scaffold (port from gui/assets prototype)
- [ ] 3.1 New `gui/` SPA root with Vite + React 18 + TypeScript; pin React + ReactDOM versions to match the prototype
- [ ] 3.2 Migrate the prototype's seven JSX files to TS modules under `gui/src/` (atoms/, app/, views/, hooks/, lib/); keep one component per file and the same exported names so reviewers can diff against `gui/assets/`
- [ ] 3.3 Reuse the prototype's `styles.css` verbatim under `gui/src/styles.css`; no Tailwind, no PostCSS-class-only refactor — the design tokens live in `:root` and `[data-theme="light"]` exactly as drafted
- [ ] 3.4 TanStack Router for the route map (overview, timeline, memory, decisions, decisions/{id}, laws, laws/{id}, laws/new, analyses, analyses/{id}, tools, graph, settings/trust)
- [ ] 3.5 TanStack Query for every list/detail endpoint plus a custom `useSSE` hook for the timeline stream
- [ ] 3.6 Replace the prototype's `MOCK` with typed fetchers; keep `gui/src/lib/fixtures.ts` (lifted from `data.js`) as Storybook fixtures + integration test seeds

## 4. Atoms + design system (preserve prototype)
- [ ] 4.1 Port `Icon`, `Sparkline`, `SeverityBar`, `Tag`, `fmtNum`, `sevTone` from `atoms.jsx` to `gui/src/atoms/`; one file per atom
- [ ] 4.2 Port `Tweak{Section,Toggle,Radio,Slider}` from `tweaks-panel.jsx` and the `useTweaks` hook (with localStorage persistence)
- [ ] 4.3 Inspector drawer + backdrop preserved verbatim (slide animation, ESC close, click-outside close)
- [ ] 4.4 Status pill (`ingesting · 312 eps` / `paused`) reads `eps` from `/v1/dashboard/overview` polled every 5 s

## 5. Views (port from gui/assets/views-*.jsx)
- [ ] 5.1 Timeline (`/timeline`) — virtualised list, ~200 events in DOM, `is-new` flash on incoming SSE rows, filter chips + search, pause/resume button (preserve eps counter + SSE state pill)
- [ ] 5.2 Memory (`/memory`) — faceted browser (kind chip group + free-text), card grid layout from `MemoryView`
- [ ] 5.3 Decisions (`/decisions` + `/decisions/{id}`) — list + supersession chain renderer (the `supersede-chain` element with arrow segments), `Show superseded` toggle, candidate-promotion call-to-action
- [ ] 5.4 Laws (`/laws` + `/laws/{id}` + `/laws/new`) — stats grid, sortable law table, trust score grid (model × repo with oklch heat shading), authoring split-pane (Monaco frontmatter + body + detector source) with `Lint` + `Publish`
- [ ] 5.5 Analysis (`/analyses` + `/analyses/{id}`) — analysis-card layout from `AnalysisView` (panelist column + verdict + decision link), live SSE stream renderer for in-progress runs
- [ ] 5.6 Tools (`/tools`) — tool usage table with bar+share visualisations, day × hour heatmap (preserve the oklch intensity formula from `view-late.jsx`)
- [ ] 5.7 Graph (`/graph`) — inline SVG renderer with `<defs>` arrow marker, zoom toolbar, legend, selection sidecard; reads from `/v1/dashboard/graph?session_id=...`
- [ ] 5.8 Trust (`/settings/trust`) — table view; renders empty state until spec 14 lands

## 6. Tweaks + UX + a11y + theming (preserve prototype behaviour)
- [ ] 6.1 Tweaks panel slides in from the right; persists to `localStorage` under `cortex.tweaks`; broadcasts changes via the `useTweaks` hook
- [ ] 6.2 Theme toggle in header AND in Tweaks; honours `prefers-color-scheme` on first visit
- [ ] 6.3 Accent hue picker — five preset chips (amber/green/blue/purple/red) plus a 20-320° slider; updates `--accent-h` CSS variable
- [ ] 6.4 Density slider — `1..10` maps to `--header-h: calc(52px - (10 - density) * 0.8px)` per the prototype
- [ ] 6.5 Sidebar collapse persists; `app.collapsed` grid template kicks in via the data attribute already in the CSS
- [ ] 6.6 Keyboard nav reaches every primary action; focus outlines visible
- [ ] 6.7 Color-blind-safe severity palette (the prototype already uses critical-red + warn-amber + info-blue)
- [ ] 6.8 Markdown renderer preserves heading order
- [ ] 6.9 Live regions for SSE updates announce severity-critical events politely
- [ ] 6.10 Lighthouse a11y score ≥90 on Timeline view

## 7. Resilience
- [ ] 7.1 SSE reconnect ladder (1 s, 2 s, 5 s, 10 s, 30 s) + stale indicator in header (re-uses the `is-paused` state pill style)
- [ ] 7.2 401 → API-key modal (with `localStorage` `cortex.api_key`)
- [ ] 7.3 429 → inline rate-limit banner + auto-retry honouring `Retry-After`
- [ ] 7.4 Slow-query (>1 s) skeletons + cancellation on route change
- [ ] 7.5 Graph editor blocks Cypher writes server-side; toast on attempt

## 8. Observability
- [ ] 8.1 Backend metrics: `cortex.dashboard.requests.total{view}`, `cortex.dashboard.sse.connections`, `cortex.dashboard.sse.dropped`, `cortex.dashboard.lint.runs{outcome}`
- [ ] 8.2 Client RUM beacons aggregated into `cortex.dashboard.rum.*` (page views, query latencies, SSE reconnect counts)

## 9. Tail (mandatory)
- [ ] 9.1 Update or create documentation covering the implementation — flip `docs/specs/16-dashboard.md` status to 🟢 + update the row in `docs/specs/00-index.md`; ship `gui/README.md` covering install / dev-server / API-key bootstrap; document the divergences from the original spec (raw CSS instead of Tailwind, inline SVG graph instead of Nexus embed, Tweaks panel as a power-user surface)
- [ ] 9.2 Write tests covering the new behavior — backend integration tests (one per endpoint, mocked SSE source, lint-blocked publish, ACL deny, RUM beacon round-trip); SPA Playwright suite covering every view (cold boot, filter round-trip, SSE reconnect, dark mode toggle, accent hue + density tweaks, keyboard navigation, API-key flow, law authoring round-trip, graph write rejection); visual regression diff against the `gui/assets/` prototype on the seven primary views; Lighthouse a11y assertion on Timeline
- [ ] 9.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `cargo test -p cortex-api`, plus `pnpm test`, `pnpm lint`, `pnpm lighthouse` for the SPA
