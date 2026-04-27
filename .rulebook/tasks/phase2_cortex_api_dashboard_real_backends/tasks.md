## 1. Phase A — quick win: archive-fed in-memory lane
- [x] 1.1 Confirm `cortex-ingestion` archive root + per-hour zstd-NDJSON layout — found at `C:/Users/Bolado/.cortex/archive/events/year=2026/month=04/day=27/hour={17,18,19}/raw-00000.parquet` (3 files; despite the `.parquet` suffix the bytes are zstd-compressed NDJSON, per `archive_loader.rs:4-6`)
- [x] 1.2 Restart `cortex-api` with `CORTEX_ARCHIVE_ROOT=…` — boot log reports `files_visited=3 envelopes_parsed=992 hits_seeded=992 lines_dropped=0`. Persisted as `CORTEX_ARCHIVE_ROOT` (+ `CORTEX_ARCHIVE_REFRESH_SECS=30` + `CORTEX_NEXUS_URL`) in `.env` so future restarts pick it up automatically.
- [x] 1.3 Probe `/v1/dashboard/overview` — `events_total=992`, `repos_indexed=2`, `kind_breakdown=[tool_call=968, turn=24]`, `recent_repos=[Cortex=724, Nexus=268]`
- [x] 1.4 Confirm GUI shows live counters — `/v1/dashboard/sessions` returns the active Claude Code session (title `"continue 2026-04-27T17-45-06.md"`, 484 events, 2h7min duration); `/v1/dashboard/tools/stats` returns `Bash=559, Read=170, Edit=147, TodoWrite=…` with real shares

## 2. Phase B — Meili-backed KeywordLane
- [ ] 2.1 Add `MeiliKeywordLane` to `crates/cortex-api/src/lanes.rs` implementing `KeywordLane` against `cortex_fulltext::MeiliClient`. Honors per-project routing, escapes user input, caps `limit` at 200.
- [ ] 2.2 `crates/cortex-api/src/main.rs` reads `CORTEX_FULLTEXT_MEILI_URL` + `CORTEX_FULLTEXT_MEILI_API_KEY` and constructs `MeiliKeywordLane`; falls back to `MemoryKeywordLane` otherwise.
- [ ] 2.3 Refactor `DashboardState` so handlers consume `Arc<dyn KeywordLane>` instead of the concrete `MemoryKeywordLane`. Tests can drive any handler with the mock or Meili variant interchangeably.
- [ ] 2.4 Unit tests against `MemoryMeiliClient` (already in cortex-fulltext) for every method on `MeiliKeywordLane`.

## 3. Per-handler wiring
- [ ] 3.1 `overview` — aggregate `meili.indexes/_/stats` across `cortex-*-{family}` for events_total / kind_breakdown / recent_repos
- [ ] 3.2 `timeline_recent` — multi-index search across `cortex-*-turns` + `cortex-*-code` sorted by `ts:desc`, honors `?limit=`
- [ ] 3.3 `memory` — search `cortex-*-misc` filtered by `kind=memory`
- [ ] 3.4 `decisions` + `decision_detail` — search `cortex-*-decisions` then enrich detail via Nexus `MATCH (d:Decision {id:'<escaped>'})`
- [ ] 3.5 `laws` / `violations` / `analyses` — search `cortex-*-governance` filtered by kind
- [ ] 3.6 `tools_stats` — aggregate `cortex-*-code` filtered by `kind=tool_call`, project `ext.tool_call.tool_name` into heatmap
- [ ] 3.7 `sessions` — Nexus `MATCH (s:Session) RETURN s ORDER BY s.id DESC LIMIT $n`
- [ ] 3.8 `graph` — already wires Nexus; widen `MATCH` to surface HAS_TURN / HAS_TOOL_CALL / TOUCHED once upstream produces them at scale

## 4. Per-project Meili settings
- [ ] 4.1 `crates/cortex-fulltext/src/indexer.rs` calls `ensure_index` lazily on first upsert to a never-seen index (currently only legacy `cortex-{family}` get settings)
- [ ] 4.2 Drop existing per-project indexes that lack settings; the next upsert recreates them with the spec-08 schema
- [ ] 4.3 Confirm `sort: ["ts:desc"]` and `filter` queries against `cortex-cortex-code` succeed (currently fail with `Attribute ts is not sortable`)

## 5. Live verification
- [ ] 5.1 Start the full stack (adapter + classifier + graph + fulltext + embedder + api)
- [ ] 5.2 Run a few tool_calls inside Claude Code; watch `/v1/dashboard/overview events_total` increment in real time
- [ ] 5.3 Open the GUI — header counters / timeline / sessions panels show real data

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation — extend spec-16 with §Backends section codifying the Meili + Nexus split; mark `MemoryKeywordLane` as test-only
- [ ] 6.2 Write tests covering the new behavior — `MeiliKeywordLane` unit tests + per-handler integration tests using `MemoryMeiliClient` + `MemoryNexusClient`
- [ ] 6.3 Run tests and confirm they pass — `cargo test -p cortex-api -p cortex-fulltext`, `cargo clippy -p cortex-api -p cortex-fulltext --all-targets -- -D warnings`
