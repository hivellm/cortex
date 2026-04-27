# 16 — Dashboard (views + SSE wiring)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 11, 14

## Goal

A single web UI that surfaces everything Cortex captures: live session timeline, memory browser, decision register, law dashboard, analysis library, tool analytics, and a Nexus graph explorer. Read-only first; a minimal authoring surface for Laws and human-mode Decisions lands second. Reuses Vectorizer's dashboard scaffold so we don't rebuild a React shell.

## Scope

**In:**
- `cortex-dashboard/` React + TypeScript SPA (Vite; scaffold forked from `Vectorizer/dashboard`).
- Backend endpoints on `cortex-api`: live SSE stream, aggregated list/detail for each view.
- Seven core views: Timeline, Memory, Decisions, Laws, Analyses, Tool analytics, Graph explorer.
- Filters: repo, model, topic, severity, time window.
- Minimal authoring: create/edit Law (draft → lint → publish), record human-mode Decisions.
- Auth: single API key (v1); OIDC placeholder for later.

**Out:**
- Mobile / responsive polish beyond "it works on a laptop."
- i18n (English only v1).
- Multi-tenant workspace switching (HivehubCloud phase).
- Full visual editor for Cypher (use the existing Nexus explorer embed).

## Inputs / Outputs

### SPA route map

| Path                      | View                                 |
|---------------------------|--------------------------------------|
| `/`                       | Overview (today's activity, counters) |
| `/timeline`               | Live session timeline (SSE)           |
| `/memory`                 | Faceted memory browser                |
| `/decisions`              | Decision register                     |
| `/decisions/:id`          | Decision detail + supersession graph  |
| `/laws`                   | Law dashboard                         |
| `/laws/:id`               | Law detail + violations history       |
| `/laws/new`               | Law authoring (draft → lint → publish)|
| `/analyses`               | Analysis library                      |
| `/analyses/:id`           | Transcript + Decision view (live SSE when open) |
| `/tools`                  | Tool analytics (heatmap, failures)    |
| `/graph`                  | Graph explorer (Nexus embed)          |
| `/settings/trust`         | Trust-score table                     |

### Backend endpoints (on `cortex-api`)

```
GET  /v1/dashboard/overview
GET  /v1/dashboard/timeline/stream        SSE — live events (architecture §5.6)
GET  /v1/dashboard/memory?facets=...
GET  /v1/dashboard/decisions
GET  /v1/dashboard/decisions/{id}
GET  /v1/dashboard/laws
GET  /v1/dashboard/laws/{id}
POST /v1/dashboard/laws                    # create draft
POST /v1/dashboard/laws/{id}/publish       # run lint + publish
GET  /v1/dashboard/analyses
GET  /v1/analysis/{id}/stream              # already in spec 15; dashboard consumer
GET  /v1/dashboard/tools/stats?since=...
GET  /v1/dashboard/trust
```

All JSON; SSE uses `text/event-stream`.

### SSE event envelope

```
event: timeline.turn.user
id: 01HY...
data: { "turn_id": "...", "session_id": "...", "ts": 1713369600000, "prompt_preview": "...", "repo": "Vectorizer" }

event: timeline.tool_call
data: { ... }

event: timeline.law_violation
data: { ... }
```

One named event per ingestion family; clients subscribe by filter (`?repo=...&severity=...`).

## Backends

The 12 `/v1/dashboard/*` endpoints share a single `MemoryKeywordLane` populated by **two loaders that run on `cortex-api` boot** (and then on a periodic refresh). Layered this way the dashboard surface stays decoupled from any one backend, and individual handlers can opt in to richer per-source paths (Nexus Cypher for the graph endpoint, future Meili pagination for high-cardinality views) without re-wiring the lane.

### Loader 1 — `archive_loader.rs`

- Source: `cortex-ingestion`'s zstd-NDJSON archive at `CORTEX_ARCHIVE_ROOT/events/year=YYYY/month=MM/day=DD/hour=HH/raw-NNNNN.parquet` (the `.parquet` suffix is historical; the bytes are zstd-compressed line-delimited JSON).
- Projects only `Kind::{Turn, ToolCall, AgentCall}` envelopes — those are the only ones the live capture surface (cortex-adapter-claude → /v1/events) writes.
- Stamps `session_id` on `LaneHit.extras` so `/v1/dashboard/sessions` and the sidebar can group/filter by session without re-reading the archive.
- Refreshes every `CORTEX_ARCHIVE_REFRESH_SECS` (default `30`).

### Loader 2 — `meili_loader.rs`

- Source: every `cortex-{slug}-{family}` Meili index in scope (per spec-08 §Routing matrix). Walks `decisions / governance / misc / turns` family suffixes only.
- Decodes the JSON-encoded envelope `body` field that `cortex-fulltext-worker` stores and hoists meaningful fields onto `LaneHit.extras`:
  - decisions → `title / status / supersedes / body_markdown`
  - law_violations → `title / law_id / body_markdown`
  - memories → `title / body_markdown`
  - analyses → `title / body_markdown` (uses `verdict` as the body when present)
- Seeds under `cortex-meili-{decisions,governance,misc,turns}` aliases so the lane keeps clean snapshots per family — flat-chains with the archive loader's `cortex-code` seed without double-counting.
- Refreshes every `CORTEX_MEILI_REFRESH_SECS` (default `60`).
- Skipped silently when `CORTEX_FULLTEXT_MEILI_URL` is unset so cold-stack dev still works with just the archive loader.

### Per-handler routing through the lane

`dashboard.rs::collect_lane_hits` aggregates **every** seeded index of the lane and the per-handler filters work against the unified hit set:

| Handler | Filter |
|---|---|
| `/overview`, `/timeline/recent`, `/sessions` | every hit (group by `symbol_to_kind`, `session_id`) |
| `/memory` | `symbol = turn / tool_call / agent_call / decision / analysis` |
| `/decisions`, `/decisions/{id}` | `symbol = "decision"`; merges `extras.{title,status,supersedes,body_markdown}` over the legacy text-scrape fallback |
| `/laws` | `symbol = "law_violation"` deduped by `extras.law_id` (until spec-13 ships a canonical catalogue source) |
| `/violations` | `symbol = "law_violation"`; surfaces `extras.law_id` and `severity = critical → action = blocked` |
| `/analyses` | `symbol = "analysis"` |
| `/tools/stats` | hits with a `tool_call:<name>` symbol; aggregates calls and the 7×24 weekday × hour heatmap |
| `/trust` | empty until spec-14 ships the `(model, repo)` derivation pipeline — handler returns honest empty arrays |
| `/graph` | bypasses the lane: when `CORTEX_NEXUS_URL` is set, runs `MATCH … RETURN nodes, edges` Cypher; otherwise falls back to a synthetic Session→Turn→ToolCall layout from the lane |

`MemoryKeywordLane` is **not** test-only — it's the production cache layer. The orchestrator (`spec-11`) consumes the same trait, so a future `MeiliKeywordLane` impl can drop in behind the same surface for high-cardinality search without changing any handler.

### Per-project Meili settings (spec-08 ↔ spec-16 contract)

`cortex-fulltext-worker` ensures the legacy `cortex-{family}` indexes at startup. Per-project uids (`cortex-{slug}-{family}`) materialise on the first upsert; `MeiliFulltextIndexer::ensure_settings` applies the spec-08 settings (sortable / filterable / searchable / ranking rules) lazily so `sort: ["ts:desc"]` and `filter: ["kind = tool_call"]` queries from the dashboard succeed against every project's index. Without this every per-project index would auto-create with an empty settings doc and the timeline / tools / decisions queries would error with `"Attribute ts is not sortable"`.

## Design

### Component layout

```
cortex-dashboard/
├─ package.json
├─ vite.config.ts
├─ src/
│  ├─ main.tsx
│  ├─ app/
│  │  ├─ routes.tsx          (TanStack Router)
│  │  ├─ api.ts              (typed fetchers, SSE client)
│  │  ├─ auth.ts             (API-key handling)
│  │  └─ theme.ts
│  ├─ views/
│  │  ├─ Overview/
│  │  ├─ Timeline/
│  │  ├─ Memory/
│  │  ├─ Decisions/
│  │  ├─ Laws/
│  │  ├─ Analyses/
│  │  ├─ Tools/
│  │  └─ Graph/
│  ├─ components/            (shared: Filters, Facets, Card, DiffView, Markdown, …)
│  └─ hooks/                 (useSSE, useFilters, usePagedQuery, …)
└─ public/
```

Vectorizer dashboard scaffold provides: TanStack Router, TanStack Query, Tailwind, a Nexus-graph embed component, Markdown renderer, and a Monaco editor. We import these — we do not fork code beyond Cortex-specific views.

### Live timeline

- SSE connection on `/v1/dashboard/timeline/stream`.
- Client-side virtual list (windowed render; keeps ~200 most recent events in DOM).
- Filter chips (repo, model, severity) change the `?...` query string; the connection re-subscribes.
- Per-event row shows: timestamp, session truncation, kind badge, repo, preview text, expand-chevron for raw payload (redacted view).
- Pause / resume button — pauses the visual feed without closing the SSE (buffer grows server-side up to 5 s, then drops oldest).

### Memory browser

- Faceted list: **repo / model / kind / topics / severity / time**.
- Backed by `/v1/dashboard/memory?facets=...`, which calls `cortex-api`'s query endpoint (spec 11) with `intent=free_search` and server-side RRF.
- Sortable by relevance or recency.
- Row click → detail pane (right-side drawer) with linked graph neighbors, related Decisions, full payload (redacted).

### Decision register

- List view with filters: repo, status (`proposed` / `accepted` / `superseded`), date.
- Detail view:
  - Body (Markdown, rendered).
  - Supersession chain as a small graph (nodes from Nexus; layout: vertical).
  - Linked Analysis (if any) + "sibling" Analyses.
  - Linked Turns / Violations.
- Diff against superseded parent when applicable.

### Law dashboard

- Card grid: one card per Law with id, title, severity badge, violation count (last 7d), enforcement tier.
- Detail view:
  - Markdown body.
  - Frontmatter (pretty-rendered).
  - Detector source (Monaco, read-only).
  - Violation history (chart + table).
  - Recent reminders emitted.
  - Supersession chain.
- Authoring:
  - `new` route → split pane: Monaco (frontmatter + body on left, detector TS on right).
  - `Lint` button → hits `/v1/dashboard/laws/{id}/lint` (wraps spec 13 linter).
  - `Publish` disabled until lint passes.

### Analysis library

- List: question, status, panel, judge mode, opened/closed.
- Detail:
  - If `status=in_progress`: SSE stream view with round-by-round progressive rendering.
  - If `status=resolved`: transcript as a collapsible thread, Decision at the top.
  - Citations rendered as clickable links to Decisions / files / prior Analyses.
  - Re-open button (guarded; creates a superseding Analysis).

### Tool analytics

- Heatmap: tool × hour-of-day → count.
- Top-N: slowest tools (P95), failed tools (error rate), most expensive (token cost).
- Per-tool drill-down: time-series of latency and success rate.
- All queries go through `/v1/dashboard/tools/stats` which rolls up from event-archive Parquet (spec 02) via DuckDB for speed.

### Graph explorer

- Thin wrapper around the Nexus-provided graph UI.
- Preset queries (dropdown): "Decisions around Vectorizer HNSW", "Law violations last 24h", "Session tree for {session_id}", "Artifact neighborhood for {file}".
- Custom Cypher editor (guarded: read-only queries only; the API rejects writes).

### Auth

- Single API key stored in `localStorage` (`cortex.api_key`).
- API key is sent as `Authorization: Bearer <key>`.
- First-time visit → modal asking for the key; `cortex-api` creates one at install (`cortex admin issue-api-key --scope dashboard`).
- OIDC hook (`onTokenAcquired`) stubbed for the future.

### Accessibility

- All interactive elements keyboard-navigable.
- Color-blind-safe badge palette (severity).
- Markdown renderer preserves heading order (no `h3` skip).
- Live regions for SSE updates announce severity-critical events politely.

### Theming

- Dark + light; respects `prefers-color-scheme`.
- One accent color per deployment (config → CSS variable).

### Failure modes

| Failure                            | UX                                                                   |
|------------------------------------|----------------------------------------------------------------------|
| SSE drops                          | Exponential reconnect (1, 2, 5, 10, 30 s); stale-indicator in header |
| API 401                            | Redirect to API-key modal                                            |
| API 429                            | Inline rate-limit banner + auto-retry                                |
| Slow query (>1 s)                  | Skeletons; cancellation on route change                              |
| Cypher write attempt in Graph view  | Backend 403; toast with "read-only"                                  |
| Law publish blocked by lint         | Inline lint output; publish button disabled with tooltip              |

### Observability (dashboard → cortex-api)

Client emits minimal RUM via a beacon endpoint (`POST /v1/dashboard/rum`): page views, query latencies, SSE reconnect counts. Aggregated into `cortex.dashboard.*` metrics.

## Acceptance criteria

- [ ] Cold `npm install && npm run dev` boots the SPA; all 7 views render against a seeded dev Cortex.
- [ ] Timeline SSE: with `cortex-adapter-claude` emitting events, events appear in <1 s; filter by `repo=Vectorizer` drops others.
- [ ] Memory browser facets: selecting `severity=critical` narrows results; URL query string reflects state; refresh preserves filter.
- [ ] Decision detail renders body as Markdown, shows supersession graph when applicable, links to parent Analysis.
- [ ] Law authoring round-trip: draft → lint (fails on intentional error) → fix → lint → publish → appears in `/laws` list.
- [ ] Published law's detector source is rendered in Monaco (read-only).
- [ ] Analysis live-stream: opening a resolved analysis shows transcript; opening an in-progress one attaches SSE and updates per round.
- [ ] Tool heatmap renders against 1 000 seeded `tool_call.*` events; drill-down for `Bash` shows success/error time-series.
- [ ] Graph explorer: preset "Decisions around Vectorizer HNSW" returns a connected subgraph; a custom write query is rejected.
- [ ] Auth flow: missing key → modal; correct key → full access; wrong key → 401 → modal again.
- [ ] Dark mode toggled from header; persists across reload.
- [ ] Keyboard-only navigation reaches every primary action; focus outlines visible.
- [ ] Lighthouse accessibility score ≥ 90 on the Timeline view.
- [ ] RUM beacon counts match actual route changes in a 5-min manual session.

## Decisions

1. **Reuse Vectorizer's scaffold.** Consistent look-and-feel across Hive tools; zero duplicated infra.
2. **SSE, not WebSocket.** Simpler, one-way is what we need; corporate proxies handle SSE better.
3. **Virtualized lists by default.** Live timeline will otherwise DOM-bomb within minutes.
4. **Graph explorer is a wrapper, not a rewrite.** Nexus owns graph UX; we embed.
5. **Authoring is minimal.** Laws and Decisions are the only write paths; everything else is read-only until we have real usage data demanding more.
6. **Monaco for code/law editing.** Familiar, accessible, and already bundled by the scaffold.
7. **Client-side RUM only.** Server-side route inference would be weaker; the client already has routing state.

## Open questions

1. **Inline debate UI for active Analyses.** Should the dashboard offer a "join as human panelist" button? Possible in principle (panel grows to 4); risks derailing auto-judge reproducibility. Deferred.
2. **Cross-repo filtering.** When a session spans repos (rare but real), do we show a multi-chip filter or collapse to first repo? Leaning multi-chip but need real-data evidence.
3. **Embedded provider pricing.** The tools-cost tile is informative; do we display a daily spend budget like the classifier's? Revisit when `cortex-analysis` accumulates cost data.

## References

- Architecture §5.6 (Dashboard views), §5.3 (retrieval consumed by Memory).
- Spec 07 — Graph writer (data source for Graph explorer).
- Spec 11 — Query API (Memory + Decisions + Analyses).
- Spec 13 — Laws DSL (authoring target, lint consumer).
- Spec 14 — Governance engine (Trust scores table).
- Spec 15 — Deep Analysis (Analysis library + live stream).
- Vectorizer dashboard scaffold: `e:/HiveLLM/Vectorizer/dashboard`.
- TanStack Router / Query docs.
