# Proposal: phase2_cortex_api_dashboard_real_backends

## Why

The Electron GUI in `gui/` calls 12 `/v1/dashboard/*` endpoints on `cortex-api` (port 17000) and renders them as the operator dashboard. Right now every endpoint returns empty:

| Endpoint                              | Response                                                       |
|---------------------------------------|----------------------------------------------------------------|
| `/v1/dashboard/overview`              | `{events_total: 0, repos_indexed: 0, kind_breakdown: [], …}`   |
| `/v1/dashboard/timeline/recent`       | `[]`                                                           |
| `/v1/dashboard/memory`                | `[]`                                                           |
| `/v1/dashboard/sessions`              | `[]`                                                           |
| `/v1/dashboard/decisions`             | `[]`                                                           |
| `/v1/dashboard/laws / violations / analyses` | `[]`                                                    |
| `/v1/dashboard/tools/stats`           | `{tools: [], heatmap: …}`                                      |
| `/v1/dashboard/trust`                 | `{models: [], repos: [], scores: {}}`                          |
| `/v1/dashboard/graph`                 | only the synthetic `session-active` node                       |

…even though the underlying stores hold real data captured today: Meili 14 722 docs across 12 indexes (cortex / vectorizer / rulebook), Nexus 1847 nodes + 9554 IN_REPO + 25 REMEMBERS edges, 3173 live events captured by `cortex-adapter-claude` from the active Claude Code session.

Root cause: `cortex-api/src/dashboard.rs::DashboardState` carries a single `Arc<MemoryKeywordLane>` (an in-memory mock from `lanes.rs:184-214`) and a single optional `Arc<NexusClient>`. The mock lane is populated only by `archive_loader` when `CORTEX_ARCHIVE_ROOT` is set — and even then, it only sees what `cortex-ingestion` archives, not the per-project Meili indexes the spec-08 router populates. None of the handlers query Meilisearch or Vectorizer at all. The GUI shows everything inactive because **the API is structurally disconnected from production stores**.

This is the spec-16 §0 MVP stub still in place — never replaced once spec-06/07/08 indexers landed.

## What Changes

- **Quick win first** (Phase A): set `CORTEX_ARCHIVE_ROOT` so the existing `archive_loader` populates the in-memory lane from the zstd-NDJSON archive `cortex-ingestion` already writes. Restores partial GUI activity in minutes.
- **New `MeiliKeywordLane`** (Phase B): real `KeywordLane` implementation backed by `cortex_fulltext::MeiliClient`. Searches `cortex-{slug}-{family}` indexes per the spec-08 routing matrix.
- `cortex-api/src/main.rs` swaps `MemoryKeywordLane` for `MeiliKeywordLane` when `CORTEX_FULLTEXT_MEILI_URL` is set; falls back to the in-memory variant otherwise so cold-stack dev still works.
- `dashboard.rs::overview` aggregates Meili `numberOfDocuments` per index → `events_total`, distinct slug prefixes → `repos_indexed`, family suffix → `kind_breakdown`. `recent_repos` ranked by total docs across each repo's family suffixes.
- `dashboard.rs::timeline_recent` searches Meili sorted by `ts:desc` across `cortex-*-turns` + `cortex-*-code`. Honors `?limit=` query string (default 50, cap 200).
- `dashboard.rs::sessions` runs Cypher `MATCH (s:Session) RETURN s ORDER BY s.id DESC LIMIT $n` against Nexus + projects each row through a `query_nexus_graph`-style mapper. `/v1/dashboard/decisions/{id}` similarly via `MATCH (d:Decision { id: '<escaped>' })`.
- `dashboard.rs::memory` searches `cortex-*-misc` filtered by `kind=memory`.
- `dashboard.rs::decisions` / `laws` / `violations` / `analyses` likewise filter `cortex-*-decisions` / `cortex-*-governance` indexes.
- `dashboard.rs::tools_stats` aggregates from `cortex-*-code` filtered by `kind=tool_call`, projects `ext.tool_call.tool_name` into the per-tool heatmap.
- `dashboard.rs::trust` stays as-is (fed by spec-12 derivation pipeline that does not ship yet) but the empty shape is now an *honest* empty rather than a mock-empty.
- `dashboard.rs::graph` already wires Nexus when `CORTEX_NEXUS_URL` is set; we widen the `MATCH` to include the new edge types (HAS_TURN / HAS_TOOL_CALL / TOUCHED) once the upstream classifier subscription gap is closed.
- **Per-project Meili settings**: ensure every per-project index has spec-08 settings (searchable / filterable / sortable). The live indexer currently only applies settings to the legacy `cortex-{family}` indexes. Settings are required for `tools_stats` / `timeline_recent` to do `sort: ['ts:desc']` and faceted aggregation.

## Impact

- Affected specs: spec-16 (dashboard backend) — codify the Meili + Nexus wiring as the production path; mark `MemoryKeywordLane` as test-only.
- Affected code:
  - `crates/cortex-api/src/lanes.rs` — new `MeiliKeywordLane`.
  - `crates/cortex-api/src/dashboard.rs` — every handler consumes the lane through the `KeywordLane` trait.
  - `crates/cortex-api/src/main.rs` — wire the Meili client; keep optional Nexus.
  - `crates/cortex-fulltext/src/meili_client.rs` — `ensure_index` for per-project indexes (lazy, on first upsert) so settings are present.
  - integration tests under `crates/cortex-api/tests/`.
- Breaking change: NO for callers (HTTP shape stays the same; the body becomes non-empty).
- User benefit: the GUI starts showing captured session data without further operator action; `/v1/query` lane wiring also feeds from the same Meili lane so keyword search across captured turns / tool_calls works end-to-end.
