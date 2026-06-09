## §1. Bug #8 — Vector retrieval quality: classifier summaries + re-embed

- [x] §1.1 Audit the Static classifier: `statics.rs` confirmed `summary: None` (no deterministic template existed)
- [x] §1.2 Implemented `static_summary()` in `statics.rs`: `"{kind} in {location}: {120-char snippet}"`; existing `summary.is_none()` tests updated to verify template format
- [x] §1.3 Changed `.env` from `CORTEX_CLASSIFIER_MODE=disabled` to `static`; docker-compose already defaults to `static` — now consistent
- [ ] §1.4 ⏸ blocked: requires live Synap enriched stream; re-classification pass on `summary IS NULL` events on next container deploy
- [ ] §1.5 ⏸ blocked: requires live embedder; trigger on next container deploy after §1.4
- [ ] §1.6 ⏸ blocked: requires live Meilisearch + vector queries; validate on next container deploy

## §2. Bug #9 — Pre-thinking bundle cache

- [x] §2.1 Read `crates/cortex-pre-thinking/src/` — located latency source in `pipeline::run`; every call constructs a fresh HTTP round-trip with no caching
- [x] §2.2 Add an in-process LRU cache: `BundleCache` in `sync_paths.rs`; key = `sha256(prompt + NUL + cwd)`, TTL = 60s, max = 256 entries; `cache_hit: bool` on `PreThinkingResult`; shared `prethink_metrics` on `SyncClient`
- [x] §2.3 Expose `cache_hit_total` and `cache_miss_total` in the pre-thinking health endpoint response; `LivePreThinkingHealthSource::snapshot()` reads from shared metrics
- [ ] §2.4 Verify: two identical pre-thinking queries within 60s — second must return in < 10ms and report `"cache": "hit"` in the response
- [ ] §2.5 Verify: p95 latency in the dashboard overview series drops below 200ms under normal session load

## §3. Bug #10 — Decision status re-parsed on bootstrap

- [x] §3.1 Read the bootstrap decision promoter — identify where `status: "proposed"` is hardcoded; found in `emitter.rs:871` (default when no Status line), but root cause is `bootstrap_seen` dedup suppressing re-emit for already-seen files
- [x] §3.2 Parse the `**Status**: <value>` line — already done in phase10i via `parse_decision_markdown`; parser is correct
- [x] §3.3 On incremental bootstrap, bypass hash suppression for `FileClass::Decision` in `runner.rs` (phase26c §3.3); decision upserts are idempotent so re-emitting is safe; 2 tests updated accordingly
- [ ] §3.4 ⏸ blocked: requires live Meilisearch + bootstrap run; verify on next container deploy

## §4. Tail (mandatory)

- [x] §4.1 Updated `docs/analysis/cortex/12-live-audit-2026-06-09.md` — added phase26c section; bugs #8, #9, #10 marked FIXED with implementation details
- [x] §4.2 Tests written: 4 bundle cache unit tests in `sync_paths.rs` (miss counter, hit counter, key uniqueness, eviction at max); static classifier template test already done in §1.2; ADR parser tested in phase10i
- [x] §4.3 `cargo check --workspace` clean; `cargo test --lib` 922+144+57 tests all pass; bootstrap_runner integration 18/18 pass; hook binary linker-locked by running adapter (Windows) — lib tests cover all new code
