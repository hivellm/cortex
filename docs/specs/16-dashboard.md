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
| `/memory`                 | Faceted memory browser (phase11j: consolidations lane with `Grain` / `Depth` / `Model` filter chips, sortable by date or `consolidation_id`; phase11r: TopicCards lane with `topic_slug` / `revision` / `confidence` / `synthesis_age_d` / `contradictions_count` / `synthesis_model` filter chips, sortable on the same axis fields — driven by the v6 settings push that makes the `ext.topic_card.*` keys filterable + sortable) |
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
GET  /v1/dashboard/stream                 SSE — dashboard delta events (spec 21)
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
GET  /v1/dashboard/tasks                   # rulebook task list (active + archived)
GET  /v1/dashboard/tasks/summary           # aggregate counters
GET  /v1/dashboard/tasks/{id}              # full proposal + sectioned checklist
GET  /v1/retention/sweeps?limit=N&since=   # phase9i — recent retention_sweeps rows + per-stage breakdown
GET  /v1/retention/state                   # phase9i — archive bytes by age bucket + cas totals + scheduled next-runs
GET  /healthz                              # legacy aggregate liveness; kept as a passthrough
GET  /v1/health                            # phase8a aggregate liveness + per-subsystem state
GET  /v1/health/freshness                  # phase8b per-(stage, repo) gap-seconds histogram
GET  /v1/health/divergence                 # phase8b cross-store divergence rows
GET  /v1/health/versions                   # phase8c per-binary git_sha + drift table
GET  /v1/health/config                     # phase8d config-audit envelope
GET  /v1/health/stream                     # SSE — health snapshots
GET  /metrics                              # Prometheus-text counters (phase8b)
```

All JSON; SSE uses `text/event-stream`.

#### Health view route inventory (phase10g)

The five `/v1/health/*` routes mount on the dashboard-aware
router (`build_router_with(service, Some(dashboard))`) — they
share the same `loader_metrics` Arc the dashboard uses, so the
freshness aggregator reads the in-process counters without a
self-fan-out. The legacy `/healthz` keeps working as an
aggregate liveness passthrough; pre-phase10g operators using
that endpoint do not need to change anything.

Regression guard:
[`crates/cortex-api/tests/health_freshness.rs::every_v1_health_route_is_mounted_on_router_with_dashboard`](../../crates/cortex-api/tests/health_freshness.rs)
hits all five routes in one test so a future refactor that
drops the `merge()` call surfaces in CI before the operator
sees blank health bodies. Operator-side, `cortex-ops doctor`
adds an `api/v1/health/*` probe against `CORTEX_API_URL` —
missing routes show as `fail` in the doctor output.

### Phase3 — `TimelineEvent` extensions

`/timeline/recent` and the SSE stream both emit `TimelineEvent` rows. As of phase3 (`phase3_tool_call_hash_preview`) every row carries three new optional fields, all `#[serde(skip_serializing_if = "Option::is_none")]` so non-tool_call rows stay lean:

- `content_hash: Option<String>` — `sha256:<64hex>` fingerprint stamped by the spec-18 capture plugin. Pass-through from `LaneHit.content_hash`. Redacted hits intentionally drop the field (see `redaction.rs`).
- `preview: Option<String>` — un-clipped tool-call body. Capped at **8 KiB** (`PREVIEW_BYTE_CAP`); rows whose source body exceeded the cap also set `preview_truncated = true` and the GUI fetches the full body via `/v1/dashboard/timeline/{id}`. The field is only populated for `kind == "tool_call"`; turns / agent_calls keep `preview = None` because the row's `detail` already covers them.
- `preview_truncated: bool` — `true` when the original body was clipped to fit `PREVIEW_BYTE_CAP`.

`/timeline/recent` and `/timeline/stream` accept a `?content_hash=<full sha256:hex>` query parameter that filters rows to the supplied fingerprint. The filter powers the Inspector's "show every call with this fingerprint" workflow (replay-detection + dedup); paired with `Filters.content_hash` GUI-side, cleared by the existing "clear filters" button.

The `cortex doctor` consistency report emits a `tool_call_hash_coverage` block: archive scan over the last 24 h asserting ≥99 % of `tool_call` envelopes carry a non-empty `content_hash`. The probe flips `report.failed = true` below the threshold; an empty window is a "skip" (no envelopes ⇒ no claim).

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

#### Cross-link: canonical `repo` casing (phase10d)

Every dashboard handler that surfaces `repo` returns it in
**canonical lowercase** (`"cortex"`, not `"Cortex"`). The
original-case label is preserved in `LaneHit.extras.repo_label`
when it differs from the canonical form, so a future GUI release
can render `Cortex` for the user without the wire shape losing
the canonical contract. Spec 02
[§Naming canonicalization](./02-storage-layout.md#naming-canonicalization-phase10d)
is the source of truth.

#### Cross-link: `/v1/query` lane composition (phase10a)

`meili_loader.rs` seeds the dashboard's per-family
`cortex-meili-{decisions,governance,misc,turns}` lane indexes
from per-repo `cortex-{slug}-{family}` Meili uids. The same
upstream documents are also queried directly by the orchestrator
via the **global** indexes (`cortex_decisions`, `cortex_turns`,
`cortex_laws`) for the `decision_lookup` / `similar_problems` /
`law_check` query intents — see
[spec 11 §Lane composition per intent](./11-query-api.md#lane-composition-per-intent-phase10a).
The dashboard's rendering layer is untouched; only the
orchestrator's strategies layer changed.

### Per-handler routing through the lane

`dashboard.rs::collect_lane_hits` aggregates **every** seeded index of the lane and the per-handler filters work against the unified hit set:

| Handler | Filter |
|---|---|
| `/overview`, `/timeline/recent`, `/sessions` | every hit (group by `symbol_to_kind`, `session_id`) |
| `/memory` | `symbol = turn / tool_call / agent_call / decision / analysis` |
| `/decisions`, `/decisions/{id}` | `symbol = "decision"`; merges `extras.{title,status,supersedes,body_markdown}` over the legacy text-scrape fallback. **Phase11k §1**: also reads top-level `decision_id` / `decision_title` / `decision_status` / `decision_supersedes` (filterable in settings v5) so the supersession-chain view scopes via Meili filters instead of an in-memory pass. |
| `/laws` | `symbol = "law_violation"` deduped by `extras.law_id` (until spec-13 ships a canonical catalogue source). **Phase11k §1**: top-level `law_id` / `law_severity` / `law_tier` are filterable so the catalogue can group by tier without re-parsing the body. |
| `/violations` | `symbol = "law_violation"`; surfaces `extras.law_id` and `severity = critical → action = blocked`. **Phase11k §2**: cross-repo violations land in the global `cortex_laws` index alongside the per-repo `cortex-{slug}-governance` index, so the dashboard can surface a global "active laws across the workspace" view via the dual-write contract. |
| `/analyses` | `symbol = "analysis"` |
| `/tools/stats` | hits with a `tool_call:<name>` symbol; aggregates calls and the 7×24 weekday × hour heatmap |
| `/trust` | empty until spec-14 ships the `(model, repo)` derivation pipeline — handler returns honest empty arrays plus `source: "stub_until_spec14"` so the GUI shows the right empty-state copy |

#### `/v1/dashboard/overview` — series block (phase2g)

The overview body now carries a `series` block alongside the existing counters. Helpers live in `crates/cortex-api/src/dashboard_series.rs` so the route handler stays compact.

```jsonc
{
  "events_total": 1234,
  "repos_indexed": 6,
  "kind_breakdown": [...],
  "recent_repos": [...],
  "series": {
    "events_per_min":          [u64; 20],            // 1-minute buckets, oldest-first
    "pre_thinking_p95_ms":     [u64 | null; 20],     // P95 of `extras.duration_ms`; null on empty buckets
    "violations_7d_daily":     [u64; 7],             // daily count of `kind=law_violation`
    "classifier_cost_usd_today": [f64; 24]           // hourly USD; all 0.0 today (see flag below)
  },
  "classifier_cost_unavailable_until_spec05": true   // flips to `false` once spec-05 stamps cost on the lane
}
```

`pre_thinking_p95_ms` reads `extras["duration_ms"]` — `archive_loader::envelope_to_hit` stamps that field for `ToolCall` and `AgentCall` envelopes. Turns alone do not populate it today; spec-12 owns turn-level latency stamping.

#### `/v1/dashboard/tools/stats` — heatmap matrix (phase2g)

```jsonc
{
  "tools": [{ "tool": "Edit", "calls": 42, "avg_ms": 0, "err_rate": 0.0, "share": 0.31 }],
  "heatmap": {
    "tz": "UTC",
    "days": ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    "cells": u32[7][24]   // counts per (weekday, hour) over the last 7 days
  }
}
```

Empty buckets are zeros (not `null`) so the renderer's oklch intensity formula has a clean `0..max` ramp. Hits with no timestamp are dropped — they cannot be placed honestly.

#### `/v1/dashboard/trust` — stub envelope (phase2g)

```jsonc
{
  "models": [],
  "repos":  [],
  "scores": {},
  "source": "stub_until_spec14"
}
```

Spec 14 lands the real derivation (rolling violation rate per `(model, repo)`). Until then, `source` lets the GUI render a "no data yet" empty state without conditionally hiding the section.
| `/graph` | bypasses the lane: when `CORTEX_NEXUS_URL` is set, runs `MATCH … RETURN nodes, edges` Cypher; otherwise falls back to a synthetic Session→Turn→ToolCall layout from the lane |

`MemoryKeywordLane` is **not** test-only — it's the production cache layer. The orchestrator (`spec-11`) consumes the same trait, so a future `MeiliKeywordLane` impl can drop in behind the same surface for high-cardinality search without changing any handler.

### Loader 3 — `tasks_loader.rs`

The `/v1/dashboard/tasks*` endpoints do **not** flow through the keyword lane — rulebook tasks are not envelopes, they are filesystem directories the rulebook MCP writes. `tasks_loader.rs` walks them directly:

- Source: `CORTEX_RULEBOOK_ROOT/{tasks,archive}/`. The default falls back to `<cwd>/.rulebook` so a `cargo run` from the repo root just works; an unreachable path yields empty results so cold-stack dev keeps booting.
- For each task directory it parses:
  - `id` — directory name; archived dirs strip the `YYYY-MM-DD-` date prefix (the prefix becomes `archived_at`).
  - `phase` — parsed from the id prefix via `^phase(\d+)([a-z]?)`. `phase2g_dashboard_enriched_metrics` → `("phase2g", 2, "g")`.
  - `title` — first H1 of `proposal.md` (the leading `Proposal:` is stripped); falls back to the id.
  - `summary` — first non-heading paragraph of `proposal.md`, trimmed to ~280 chars; rulebook scaffold placeholders (`[Explain why...]`) are skipped.
  - `progress` — `(done, total)` derived from the `- [x]` / `- [ ]` checkbox count in `tasks.md`.
  - `status` — read from `.metadata.json` (`pending` / `in-progress` / `completed`); forced to `archived` for anything under `archive/`.
  - `created_at` / `updated_at` — from `.metadata.json`; archive's date prefix is the fallback when the metadata file is absent.
- Caching: rows are stored behind a `RwLock`; a 30-second TTL gates full re-scans, and per-task mtime stamps invalidate individual rows when `proposal.md` / `tasks.md` / `.metadata.json` advance between scans.

Endpoints:

| Path | Behaviour |
|------|-----------|
| `GET /v1/dashboard/tasks` | Filter knobs: repeated `status=pending|in-progress|completed|archived`, repeated `phase=phase2g|phase4a`, `include_archived=bool` (default `true`), `limit` (default 200, capped at 500), `offset` (default 0), `sort=phase|updated_at|created_at` (default `phase`), `order=asc|desc`. Default sort: phase numeric asc → letter asc → `updated_at` desc. Response: `{ tasks, total, by_phase, by_status }` where `by_phase` / `by_status` are aggregated across the **unfiltered** population so the GUI can render filter chips with stable counts. |
| `GET /v1/dashboard/tasks/summary` | `{ total, completed, in_progress, pending, archived, completion_pct }` — `completion_pct = (completed + archived) / total * 100`, rounded to one decimal. |
| `GET /v1/dashboard/tasks/{id}` | Full row + `proposal_md` (full body) + `checklist` (sectioned by H2; preserves file order) + `specs` (recursive listing under the task's `specs/` directory). When the same id exists both active and archived, the active row wins and `also_archived: true` is set on the response.|

The loader is read-only; the rulebook MCP remains the only writer for `.rulebook/{tasks,archive}/`.

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
- Backed by `/v1/dashboard/memory`, which calls `cortex-api`'s query endpoint (spec 11) with `intent=free_search` and server-side RRF.
- Sortable by relevance or recency.
- Row click → detail pane (right-side drawer) with linked graph neighbors, related Decisions, full payload (redacted).

#### Kind filter (phase10f)

The handler accepts `?kind=<canonical>` (repeated to OR
multiple values) and `?facets=<canonical>` as a synonym so the
pre-phase10f GUI URL contract keeps working. Canonical set:
`turn`, `tool_call`, `agent_call`, `memory`, `decision`,
`analysis`, `law_violation`, `knowledge`, `learning`. Any other
value surfaces as `400 unknown_kind` with body
`{"error":"unknown_kind","received":"<value>","canonical":[...]}`
so callers see typos at the API boundary instead of silently
getting an empty list.

The filter applies BEFORE pagination — `?limit=2&kind=decision`
returns up to two decision rows, NOT two mixed-kind rows from
which only matching decisions are then sliced. The pre-phase10f
handler clamped first then filtered, so a small limit dropped
the requested rows entirely (the audit caught the GUI's Memory
tab showing only `tool_call`/`turn` even though the overview
reported 287 memories + 26 decisions + 33 analyses + 121
violations sitting in the same lane).

#### Claude Code archive turns (phase11i §1 + §5)

The Memory browser surfaces `Turn` envelopes ingested by
[`cortex-claude-archive`](../../crates/cortex-claude-archive/) the
same way it surfaces live-capture turns. Wire shape on the
envelope distinguishes the two paths via:

- `tool` field — `"claude-code"` for the conversation archive
  (vs. `"cortex"` for live capture, `"openai-codex"` / `"cursor"`
  / `"gemini"` once those adapters land).
- `stream` field — `Bootstrap` while the watcher is back-filling
  a session, `Live` once the live adapter takes over (the
  watcher de-dups by `event_id`, so a session that flips from
  bootstrap to live mid-stream surfaces under both stream tags
  without double-counting in the dashboard counters).

The `?tool=<name>` filter (phase11i §3.3) lets operators slice
by originating AI: `GET /v1/dashboard/memory?kind=turn&tool=claude-code`
returns only the conversation-archive turns. Same UI affordance
as the existing `?kind=` filter — repeated values OR together,
unknown values surface as `400 unknown_tool`.

The `Past sessions` overlay (phase11i §4.1) lives under the
spec-12 pre-thinking renderer rather than the Memory browser
itself, but the row-click detail pane on a `Turn` shows its
parent `session_id` and a "see related sessions" link that
re-runs `intent=similar_problems` scoped to the same
`session_cohort` so the dashboard surfaces the same neighbours
the renderer's overlay does.

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

> **Status:** 🟢 Implemented — phase3_gui_multi_connection ([proposal](../../.rulebook/tasks/phase3_gui_multi_connection/proposal.md))

Auth is **opt-in per deployment**. Localhost dev keeps zero
authentication so a fresh `docker compose up` works without
keys; remote deployments (cortex-api behind a reverse proxy,
multi-user host) flip the gate via `CORTEX_DASHBOARD_AUTH=1`
and mint keys with the daemon's admin CLI.

**Where the auth lives.** Inside the renderer's `Connection`
record, not as a login screen on the dashboard. Each connection
the user adds carries its own `auth: { kind: "none" | "bearer"
| "basic", … }`; the active connection's auth field drives the
`Authorization` header on every fetch. Switching connections
swaps the auth instantly without a re-login.

**Server-side surface (`cortex-api`).**

- `CORTEX_DASHBOARD_AUTH=1` flips the
  [`require_api_key`](../../crates/cortex-api/src/auth.rs)
  middleware on. Off (default) → middleware short-circuits
  with zero per-request cost; on → every `/v1/dashboard/*`
  request must carry a valid bearer.
- `/v1/status`, `/healthz`, `/v1/query`, `/metrics` stay
  anonymous regardless so liveness probes from operators /
  load balancers / the renderer's own polling layer do not
  need a key.
- Keys persist in `~/.cortex/api_keys.sqlite` (override path
  via `CORTEX_API_KEYS_DB`) as Argon2id digests; cleartext is
  printed exactly once at issue time.
- Constant-time compare via `subtle::ConstantTimeEq` (inside
  `argon2::verify_password`). The middleware verify path runs
  on a `spawn_blocking` task because Argon2id is CPU-bound.
- 401 body: `{"reason":"missing_or_invalid_api_key"}` —
  matches the existing `/v1/query` error envelope shape.

**Admin CLI (server-side).**

```sh
# Mint a key. Cleartext printed once; only the hash persists.
cortex-api admin issue-api-key --scope dashboard --label local-gui
# List keys (id / scope / label / timestamps; hashes never printed).
cortex-api admin list-api-keys
# Soft-revoke a key. The middleware blocks the next request that uses it.
cortex-api admin revoke-api-key <id>
```

**Renderer flow.**

1. First-time launch seeds a built-in `local` connection
   (`http://127.0.0.1:17000`, `auth.kind=none`); the dashboard
   loads against it without prompting.
2. User adds a remote connection via the manage view
   (`/connections`). Auth selector picks `bearer` or `basic`.
3. When any fetcher returns 401 from a non-localhost
   connection, [`ApiKeyPromptHost`](../../gui/src/shell/ApiKeyPrompt.tsx)
   pops a modal pasting-only ESC-locked dialog. Submit writes
   the token onto the active Connection and the next fetch
   reconnects.
4. SSE escape hatch: `EventSource` cannot carry custom
   headers, so the renderer appends `?api_key=<token>` when
   the active connection has a bearer token. The middleware
   accepts the query-param identically to the header.

**CORS.** When the renderer talks from a non-Electron browser
the daemon's CORS layer
([`dashboard_cors_layer`](../../crates/cortex-api/src/auth.rs))
defaults permissive for localhost (Vite dev server,
`http://127.0.0.1:*`, `file://`). Remote deployments override
via `CORTEX_API_ALLOWED_ORIGINS=https://cortex.example.com`.

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
| API 401 (remote connection)        | `ApiKeyPrompt` modal locked open until a key is pasted (ESC + backdrop ignored) |
| API 401 (localhost connection)     | Toast — auth gate misconfigured locally; modal does not pop          |
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

## Relevance trend (phase6e — F-008)

The dashboard renders a trend view over the relevance harness reports
persisted to `.rulebook/learnings/relevance/<YYYY-MM-DD>-<sha>.json`
(written by [`.github/workflows/relevance.yaml`](../../.github/workflows/relevance.yaml)
on every push to `main`). The contract:

- **Source of truth** — the JSON files in
  `.rulebook/learnings/relevance/`. Each file is the harness's
  `RelevanceReport` shape documented in
  [spec 11 §Relevance harness](./11-query-api.md#relevance-harness-phase6e).
- **Series** — global `recall_at_10_pct` and `mrr_avg` over time,
  plus a per-intent split for triage.
- **Drill-down** — clicking a point opens the matching report's
  `queries[]` rows (sorted by id), highlighting `recall_at_10=false`
  rows so a regression is one click from the failing query.
- **Worst-N tile** — a static tile listing the latest report's
  `omitted_intents` so operators can spot when a backend was down for
  a run (and the report's recall numbers are partial).

The view does not call `/v1/query` itself; it streams the JSON files
straight from disk so trend rendering is independent of daemon
health. New reports appear automatically as soon as the CI commit
lands on `main`.

## Open questions

1. **Inline debate UI for active Analyses.** Should the dashboard offer a "join as human panelist" button? Possible in principle (panel grows to 4); risks derailing auto-judge reproducibility. Deferred.
2. **Cross-repo filtering.** When a session spans repos (rare but real), do we show a multi-chip filter or collapse to first repo? Leaning multi-chip but need real-data evidence.
3. **Embedded provider pricing.** The tools-cost tile is informative; do we display a daily spend budget like the classifier's? Revisit when `cortex-analysis` accumulates cost data.

## Retention view (phase9i)

The Retention tab sits between Memory and Decisions in the sidebar
and consumes the two new endpoints listed above:

- `/v1/retention/sweeps?limit=N&since=RFC3339` — newest-first rows
  from `retention_sweeps` joined with the per-stage breakdown
  (`tier_transitions_json` parsed into a `stages` object). Stage
  keys mirror the sweeper module names: `sweep`, `parquet_rollup`,
  `cas_vacuum`, `pii_enforce`, `turn_digest`, `meili_prune`,
  `metadata_reap`. The GUI uses the key set to project one card
  per sweep type, including state, last-run relative time, and
  bytes reclaimed.
- `/v1/retention/state` — compact snapshot:
  - `archive_bytes` bucketed `le_30d` / `30d_to_365d` / `gt_365d`,
    plus a `total` and an `available: bool` flag (false when the
    archive root is missing — handler stays honest about cold
    boots).
  - `cas` rows + bytes (read directly from `cas.sqlite`).
  - `collections` and `meili_indexes` are present-but-empty until
    a live SDK probe is wired through `DashboardState`.
  - `next_runs` carries one row per sweep type with `next_run`
    set to `"never"` until phase9k publishes a cron schedule.

The view itself (`gui/src/views/Retention.tsx`):

- Failure banner — red bar appears when any sweep type has
  `status='failed'` in its two most recent rows; surfaces the most
  recent `last_error` from the stages payload.
- Header card row — one card per sweep type, color-coded
  ok / degraded / failed / never, last-run relative time, bytes
  reclaimed.
- 30-day reclamation sparkline derived from each row's
  `bytes_reclaimed` stage counter.
- Sortable storage breakdown table sourcing the archive +
  CAS rows (sortable by source name, size now, or delta).
- Live log strip — subscribes to `/v1/dashboard/timeline/stream`
  and filters for events whose `kind` starts with `retention.`.
  The list is capped at 100 entries and surfaces the SSE
  connected/disconnected pill.

## References

- Architecture §5.6 (Dashboard views), §5.3 (retrieval consumed by Memory).
- Spec 07 — Graph writer (data source for Graph explorer).
- Spec 11 — Query API (Memory + Decisions + Analyses).
- Spec 13 — Laws DSL (authoring target, lint consumer).
- Spec 14 — Governance engine (Trust scores table).
- Spec 15 — Deep Analysis (Analysis library + live stream).
- Spec 19 — Retention (sweepers whose history feeds the new tab).
- Vectorizer dashboard scaffold: `e:/HiveLLM/Vectorizer/dashboard`.
- TanStack Router / Query docs.

## Pure-reader contract (ADR-014 / phase13f)

Dashboard handlers under `crates/cortex-api/src/dashboard.rs` plus
the 19 submodules in `crates/cortex-api/src/dashboard/` are **pure
typed readers** of per-domain `*ReportView` projections. Concrete
contract:

1. **Every domain that surfaces dashboard state exposes a
   `Report` struct + a `view() -> Self::ReportView` projection**:
   - `cortex_workers::sweep::report::SweepReport` →
     `SweepReportView` (phase13a, ADR-009).
   - `cortex_workers::producer::report::ProducerReport` →
     `ProducerReportView` (phase13f §2.4, ADR-010).
   - `cortex_workers::producer::report::ProducerCheckpoint` →
     `ProducerCheckpointView` +
     `ProducerCheckpointsReportView` aggregator (phase13f §3.4 —
     the dashboard surfaces the durable checkpoint state since
     `ProducerReport` is in-memory only).
   - `cortex_workers::coverage::CoverageReport` →
     `CoverageReportView` with per-backend
     `BackendCoverageEntry` rows in fixed order (phase13f §2.3,
     ADR-012).
   - `cortex_api::dashboard::consolidations::ConsolidationRow`
     wrapped in `ConsolidationReportView { rows, total, filter }`
     (phase13f §2.2). The wire echo of the active filter lives on
     `ConsolidationFilter`.

2. **Handlers MUST call `.view()` rather than render the Report
   directly.** This keeps the wire format stable across
   domain-side refactors and prevents handler-side derivations of
   the kind phase11v hit (a handler renders `"never"` while the
   underlying table has a row).

3. **No fallback string sentinels in the dashboard handler
   tree.** A CI grep gate enforces this. The workflow is
   `.github/workflows/dashboard-grep-gate.yml`; it walks
   `crates/cortex-api/src/dashboard.rs` plus every `.rs` under
   `crates/cortex-api/src/dashboard/` and fails the build if it
   finds the literal `"never"`, `"n/a"`, or `"unknown"` (with
   quotes) anywhere in that tree. The pre-bucket phase13a gate
   only scanned `retention.rs` + `dashboard.rs`; phase13f §4.1
   expanded the scope to the full submodule tree.

4. **Missing state is wire-level `null` (or the typed empty
   equivalent on a `*ReportView`).** A handler MAY return an
   honest-empty view when the upstream is not wired (e.g.
   `dashboard::coverage` returns a `CoverageReportView` with an
   empty `metadata_db` marker when `DashboardState.metadata` is
   `None`). It MUST NOT invent a status string.

5. **New dashboard panels follow the same pattern:** define a
   `Report` in the owning domain crate, add `view()`, write the
   handler as `Json(report.view())`. The mechanical cost is ~50
   lines of trait impl + ~100 lines of JSX (the GUI counterpart
   is a `*ReportView`-typed consumer with no local fallback
   branches).
