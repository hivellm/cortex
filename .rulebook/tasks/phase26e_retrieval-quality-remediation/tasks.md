## §1. Gap A — purge stale raw-JSON vectors, clean re-embed (Bug #8 finish)

- [ ] §1.1 Backfill the `analyses` index summaries (phase26d found 22% coverage; turns/code/docs already 100%).
- [ ] §1.2 Add a purge path to the embedder/re-embed: re-embed must delete-then-insert per event id, not add a duplicate vector.
- [ ] §1.3 Re-embed the `cortex` repo collections through the purge path.
- [ ] §1.4 Re-measure: `/v1/query` repo=cortex query="event classification system" top vector hit must clear 0.50 (phase26d measured 0.238 with additive vectors).

## §2. Gap B — observable bundle-cache hit rate

- [ ] §2.1 Publish adapter `cache_hit_total` / `cache_miss_total` into the `cortex-adapter` subsystem `extras` (or a dedicated adapter health endpoint).
- [ ] §2.2 Verify two identical pre-thinking queries within the 60s TTL increment the live `cache_hit_total`.

## §3. Gap C — dedicated pre-thinking latency metric

- [ ] §3.1 Record bundle-assembly latency separately from envelope `duration_ms`.
- [ ] §3.2 Surface it as its own dashboard series (or repoint `pre_thinking_p95_ms` to the real source); confirm < 200ms for repeated same-scope/intent queries.

## §4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §4.1 Update or create documentation covering the implementation
- [ ] §4.2 Write tests covering the new behavior
- [ ] §4.3 Run tests and confirm they pass
