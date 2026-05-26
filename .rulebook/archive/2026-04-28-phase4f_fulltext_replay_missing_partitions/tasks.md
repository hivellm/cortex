## 1. Boot-time replay-missing routine
- [x] 1.1 Add `boot_replay::missing_partitions(client, archive_root)` returning `BTreeSet<(String, String)>` of `(repo_slug, family)` pairs present in the archive but absent from Meili
- [x] 1.2 Add `boot_replay::replay_missing_partitions(client, indexer, archive_root)` that walks the archive once and runs the `MeiliFulltextIndexer` upsert path on every event whose routing matches a missing partition
- [x] 1.3 Gate the call on `CORTEX_FULLTEXT_REPLAY_MISSING=1` in `main.rs`; default off so hot-path restarts stay fast

## 2. Metrics + observability
- [x] 2.1 Add `cortex_fulltext_replay_events_total{repo, family}` counter
- [x] 2.2 Emit a single `info` summary line at the end of the replay phase: `examined_archives=N, missing_partitions=M, replayed_events=K, latency_ms=L`

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — extend `docs/specs/08-fulltext-indexer.md` with a `### Boot-time replay-missing partitions` section that mirrors the existing `### Boot-time stale-index sweep` shape
- [x] 3.2 Write tests covering the new behavior — `MemoryMeiliClient` seeded with one canonical index + a temp archive containing events for two repos; assert that after `replay_missing_partitions` the second repo's index is created and its document count matches the archive's event count
- [x] 3.3 Run tests and confirm they pass — `cargo check -p cortex-fulltext` → `cargo clippy -p cortex-fulltext --all-targets -- -D warnings` → `cargo test -p cortex-fulltext`
