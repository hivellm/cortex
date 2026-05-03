## 1. FS watcher: add `.rulebook/learnings/**` coverage

- [ ] 1.1 Locate the phase11m FS watcher module in `cortex-api` (likely `crates/cortex-api/src/dashboard_watcher.rs` or wherever `notify-debouncer-mini` is wired); confirm the current path glob list
- [ ] 1.2 Extend the path glob to include `.rulebook/learnings/**`; add a new `DashboardEvent::LearningAdded { id, title }` variant on the bus
- [ ] 1.3 Map the watcher fire to the new event variant; reuse the existing 1024-id dedup ring keyed on `(rel_path, content_hash)` so re-saves do not double-fire
- [ ] 1.4 Unit test: drop a fixture `learnings/<id>.metadata.json`, assert one `LearningAdded` event reaches a subscribed bus consumer

## 2. SQLite tail loop for `.rulebook/memory/memory.db`

- [ ] 2.1 New module `crates/cortex-api/src/memory_tail.rs` exposing `MemoryTailWatcher` with constructor `(db_path: PathBuf, bus: Arc<DashboardEventBus>)` and `tick_once() -> TickReport`
- [ ] 2.2 Open the SQLite file with `rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI` so concurrent rulebook writes never block; reuse one `Connection` per process
- [ ] 2.3 Per-tick query: `SELECT id, name, type, COALESCE(updated_at, created_at) AS ts FROM memories WHERE rowid > ?1 ORDER BY rowid` keyed off the cached `last_rowid`; on first tick, seed `last_rowid = SELECT MAX(rowid) FROM memories` so the first tick does not replay history into the bus
- [ ] 2.4 Emit `DashboardEvent::MemoryAppended { id, name, memory_type }` per new row; update `last_rowid` only after a successful bus publish
- [ ] 2.5 Wire into `cortex-api/src/main.rs` startup alongside the FS watcher; spawn a tokio interval at 250 ms; gate behind `CORTEX_DASHBOARD_MEMORY_TAIL` env (default `1`); tolerate absence of `memory.db` (sibling repos without rulebook) by treating the file-missing case as a no-op tick
- [ ] 2.6 Integration test: write two rows to a fixture `memory.db`, assert two `MemoryAppended` events reach the bus within 1 s; assert no duplicate emit on the third tick when no new rows landed
- [ ] 2.7 Defensive: SQLite open errors / schema-mismatch errors flag the watcher degraded but never panic; log via `tracing::warn!` once per error class (rate-limit so a missing schema does not spam every 250 ms)

## 3. GUI dispatch update

- [ ] 3.1 `gui/src/hooks/useDashboardStream.ts` — extend the kind → query-key invalidation map with `learning.added` → memory + learnings query keys, `memory.appended` → memory query key
- [ ] 3.2 GUI smoke (manual or vitest mock-EventSource): emit each new event kind, assert the registered React-Query keys are invalidated exactly once

## 4. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 4.1 Update or create documentation covering the implementation — flip `docs/specs/21-dashboard-push.md` Draft → Implemented; add a "SQLite tail" subsection covering the read-only handle + 250 ms cadence + `last_rowid` seeding rule; document `CORTEX_DASHBOARD_MEMORY_TAIL` in `crates/cortex-api/README.md`; CHANGELOG entry under `[Unreleased]` Added
- [ ] 4.2 Write tests covering the new behavior — §1.4, §2.6, §3.2 land; coverage ≥ 95 % on `crates/cortex-api/src/memory_tail.rs`
- [ ] 4.3 Run tests and confirm they pass — `cargo check -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test -p cortex-api`. All green before archive.
- [ ] 4.4 Capture learning: `rulebook_learn_capture` for the "do not extend a generic upstream package with consumer-specific stream contracts" pattern (the original phase11n premise) — the consumer owns the observation surface
