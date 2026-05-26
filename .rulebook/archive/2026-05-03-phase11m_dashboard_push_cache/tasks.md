## 1. Stream contract + spec

- [x] 1.1 Write `docs/specs/21-dashboard-push.md` defining stream name (`cortex.events.dashboard`), event envelope (`event_id`, `kind`, `entity_id`, `summary`, `ts`, optional `delta`), and per-kind body shape for `task.changed`, `handoff.appended`, `decision.changed`, `memory.appended`, `knowledge.added`.
- [x] 1.2 Add Rust types in `crates/cortex-core/src/dashboard_event.rs` (`DashboardEvent`, `DashboardEventKind`, `DashboardEventSource`) — serde-tagged on `kind`; cover all kinds. Re-exported from `lib.rs`.
- [x] 1.3 Unit test (in `cortex-core`) round-tripping every event kind through serde JSON. 7/7 passing.

## 2. Publisher: file watcher fallback

- [x] 2.1 Workspace deps: pinned `notify = "8.2"` + `notify-debouncer-mini = "0.7"` in root `Cargo.toml`; consumed via `workspace = true` in `crates/cortex-api/Cargo.toml`.
- [x] 2.2 New module `crates/cortex-api/src/dashboard_watcher.rs` — `notify-debouncer-mini` 250 ms debounce, `RecursiveMode::Recursive` on `tasks`/`handoff`/`decisions`/`knowledge`. Errors with `notify::Error::generic` when no subdir exists.
- [x] 2.3 `classify(&root, &path)` derives the `DashboardEvent` kind from the changed path (`tasks/<id>/...` → `TaskChanged{id}`, etc.) and the watcher publishes via `DashboardEventBus` (wraps `tokio::sync::broadcast`, capacity 1024).
- [x] 2.4 `DashboardState::events_bus` field added; `cortex-api/src/main.rs` builds `DashboardEventBus`, spawns one watcher per rulebook root (`CORTEX_DASHBOARD_WATCH=0` opts out), and `Box::leak`s the handles for daemon lifetime. Bus is lossy (broadcast capacity 1024).
- [x] 2.5 Unit test (`watcher_emits_event_on_real_file_write`) writes a real file under `tempdir/tasks/phase1_demo/tasks.md` and asserts the matching `TaskChanged` event arrives via the bus inside 2 s.

## 3. SSE endpoint

- [x] 3.1 `dashboard_stream` handler subscribes to `events_bus`, emits `event: hello { server_ts, lost_window }`, then one SSE frame per `DashboardEvent`. `Lagged` recoveries re-emit `hello { lost_window: true }`; `Closed` ends the stream. Keep-alive 15 s.
- [x] 3.2 Route `/v1/dashboard/stream` registered alongside `timeline/stream` in `build_dashboard_router`.
- [x] 3.3 Integration test `tests/dashboard_stream.rs::stream_emits_hello_frame_then_published_event` reads the SSE body via `body.into_data_stream()`, asserts `hello` + `task.changed` frame for a published event. 1/1 passing.
- [x] 3.4 README `crates/cortex-api/README.md` — added `/v1/dashboard/stream` to endpoint list and documented `CORTEX_DASHBOARD_WATCH` / `CORTEX_DASHBOARD_PUBLISH` env vars.

## 4. Synap consumer + dedup (publisher deferred)

- [x] 4.1 Audit: `cortex-mcp-server/src/tools.rs` exposes `cortex_query`, `cortex_pre_thinking`, `cortex_status` — all read-only retrieval. The rulebook MCP that mutates `.rulebook/` lives in `@hivellm/rulebook` (external repo, out of scope for this task). Watcher path (§2) covers every file-backed mutation regardless of who wrote it. Memory + knowledge writes that are DB-only (not file-backed) need the publisher to surface in real-time — tracked by follow-up task `phase11n_rulebook_dashboard_publisher`.
- [x] 4.2 Follow-up task `phase11n_rulebook_dashboard_publisher` created for the @hivellm/rulebook-side publisher work. The local consumer (§4.3) is built ahead of it so the wiring is ready when the publisher lands.
- [x] 4.3 `crates/cortex-api/src/dashboard_consumer.rs` — `DashboardEventConsumer` exposes `ingest(Value)` (parses) and `ingest_event(DashboardEvent)` (already parsed); dedupes via `VecDeque<String>` ring (capacity 1024); surfaces `ConsumerMetrics { forwarded, deduped, parse_errors }`. Synap pull-loop intentionally not here — phase11n owns it.
- [x] 4.4 4/4 tests passing: `dedupes_repeat_event_ids` (2 unique + 1 duplicate → forwarded=2 / deduped=1), `parse_error_is_counted_not_forwarded`, `well_formed_payload_ingests_via_value`, `evicts_oldest_id_when_window_fills`.

## 5. GUI integration

- [x] 5.1 `gui/src/lib/useDashboardStream.ts` opens a single EventSource on `dashboardStreamUrl()` per `connKey`, exponential reconnect (1s → 30s ladder), exposes `{ connected, reconnects, lastEventAt }`.
- [x] 5.2 Hook called once in `App.tsx` `AppShell`. Per-kind dispatch via `keyPrefixesFor` (task.changed → `tasks` + `tasks-summary`; decision.changed → `decisions` + `decision-detail/{id}` when entity_id present; handoff/memory/knowledge → single prefix). On `hello { lost_window }` (or any reconnect hello), fires `invalidateAllDashboardQueries` to recover any drop window.
- [x] 5.3 Polling bumped 30 s → 300 s in `Tasks.tsx`, `Handoffs.tsx`, `Decisions.tsx`. Memory left at 8 s by design — its writes land in `~/.claude/projects/.../memory/`, outside `.rulebook/`, so the watcher does not observe them; pushing those events lives in phase11n once the rulebook MCP gets a Synap publisher.
- [x] 5.4 Header pill: `dashboardStream` prop drilled from App into Header; renders `stream` (green) or `stream offline` (amber) using the existing `.status-pill` class. Tooltip shows reconnect count.
- [x] 5.5 Verification §Verification added to `docs/specs/21-dashboard-push.md` — 6-step manual checklist (load Tasks view, mutate via MCP, observe row update inside 2 s, daemon restart resync) plus pointers to the three automated test entry points already passing.

## 6. Cache hydration policy

- [x] 6.1 GUI `useDashboardStream` `hello` handler dispatches `invalidateAllDashboardQueries(client, connKey)` on every reconnect-hello (and on first hello when `lost_window=true`); skips only the cold-mount `lost_window=false` case to avoid invalidating freshly-fetched queries.
- [x] 6.2 Server `dashboard_stream` first SSE frame is `event: hello` with body `{ server_ts, lost_window }`; `Lagged` recoveries re-emit the hello with `lost_window: true` so GUI subscribers self-heal.
- [x] 6.3 `gui/src/lib/useDashboardStream.test.ts` exercises the resync helper end-to-end: 3/3 vitest cases passing — kind-tag list locked against the wire, `invalidateAllDashboardQueries` fires exactly 6 invalidations (2 for tasks, 1 each for handoffs/decisions/memory/knowledge), every invalidation scoped to the active connKey.

## 7. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/21-dashboard-push.md` written (full envelope contract, kinds table, failure modes, performance targets, telemetry, verification checklist); `docs/specs/16-dashboard.md` endpoint table now lists `/v1/dashboard/stream`; CHANGELOG `[Unreleased] → Added` carries a Phase11m entry referencing spec 21 + the companion phase11n task.
- [x] 7.2 Write tests covering the new behavior — 7 unit tests in `cortex-core::dashboard_event`, 9 in `cortex-api::dashboard_watcher` (including a real-fs roundtrip), 4 in `cortex-api::dashboard_consumer`, 1 integration test `tests/dashboard_stream.rs`, 3 vitest cases in `gui/src/lib/useDashboardStream.test.ts`. Every new module is exercised end-to-end against the spec 21 wire format.
- [x] 7.3 Run tests and confirm they pass — `cargo check -p cortex-core -p cortex-api` clean; `cargo clippy -p cortex-api --no-deps --tests` clean for every new file (one pre-existing `cortex-core` doc-lazy-continuation in `events.rs:360` lives in phase11j commit `f06cdab` and is out of scope for this task); `cargo test -p cortex-core --lib dashboard_event` 7/7, `cargo test -p cortex-api --lib dashboard` 53/53, `cargo test -p cortex-api --test dashboard_stream` 1/1, `cargo test -p cortex-api --test dashboard_tasks` 6/6; GUI `npx tsc --noEmit` clean, `npx vitest run` 29/29 across 5 files.
