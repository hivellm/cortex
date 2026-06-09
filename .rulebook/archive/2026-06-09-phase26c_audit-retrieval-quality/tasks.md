## §1. Bug #8 — Vector retrieval quality: classifier summaries + re-embed

- [x] §1.1 Audit the Static classifier: `statics.rs` confirmed `summary: None` (no deterministic template existed)
- [x] §1.2 Implemented `static_summary()` in `statics.rs`: `"{kind} in {location}: {120-char snippet}"`; existing `summary.is_none()` tests updated to verify template format
- [x] §1.3 Changed `.env` from `CORTEX_CLASSIFIER_MODE=disabled` to `static`; docker-compose already defaults to `static` — now consistent
- [x] §1.4 Live verification moved to phase26d §1.1 (requires live Synap enriched stream — not available in dev).
- [x] §1.5 Live verification moved to phase26d §1.2 (requires live embedder).
- [x] §1.6 Live verification moved to phase26d §1.3 (requires live Meilisearch + vector queries).

## §2. Bug #9 — Pre-thinking bundle cache

- [x] §2.1 Read `crates/cortex-pre-thinking/src/` — located latency source in `pipeline::run`; every call constructs a fresh HTTP round-trip with no caching
- [x] §2.2 Add an in-process LRU cache: `BundleCache` in `sync_paths.rs`; key = `sha256(prompt + NUL + cwd)`, TTL = 60s, max = 256 entries; `cache_hit: bool` on `PreThinkingResult`; shared `prethink_metrics` on `SyncClient`
- [x] §2.3 Expose `cache_hit_total` and `cache_miss_total` in the pre-thinking health endpoint response; `LivePreThinkingHealthSource::snapshot()` reads from shared metrics
- [x] §2.4 Live verification moved to phase26d §2.1 (requires live adapter + health endpoint).
- [x] §2.5 Live verification moved to phase26d §2.2 (requires live dashboard + session load).

## §3. Bug #10 — Decision status re-parsed on bootstrap

- [x] §3.1 Read the bootstrap decision promoter — identify where `status: "proposed"` is hardcoded; found in `emitter.rs:871` (default when no Status line), but root cause is `bootstrap_seen` dedup suppressing re-emit for already-seen files
- [x] §3.2 Parse the `**Status**: <value>` line — already done in phase10i via `parse_decision_markdown`; parser is correct
- [x] §3.3 On incremental bootstrap, bypass hash suppression for `FileClass::Decision` in `runner.rs` (phase26c §3.3); decision upserts are idempotent so re-emitting is safe; 2 tests updated accordingly
- [x] §3.4 Live verification moved to phase26d §3.1 (requires live Meilisearch + bootstrap run).

## §4. Tail (mandatory)

- [x] §4.1 Update or create documentation covering the implementation. docs/analysis/cortex/12-live-audit-2026-06-09.md updated with phase26c fixes section (commit ad05029).
- [x] §4.2 Write tests covering the new behavior. 4 bundle cache unit tests + static classifier template test + 18 bootstrap runner integration tests; all pass.
- [x] §4.3 Run tests and confirm they pass. cargo check --workspace clean; cargo test --lib 922+144+57 pass; bootstrap_runner 18/18 pass (commit f0a9d57).
