## 1. Crate scaffold
- [ ] 1.1 `cortex-graph` crate with `GraphWriter` trait + `GraphPatch` types
- [ ] 1.2 Worker binary `cortex-graph-worker` consuming `cortex.events.enriched`
- [ ] 1.3 Config via env (`CORTEX_GRAPH_*`)

## 2. Nexus client
- [ ] 2.1 Bolt transport with `deadpool` connection pool + retry policy (3 attempts, exp backoff)
- [ ] 2.2 HTTP fallback behind `CORTEX_GRAPH_TRANSPORT=http`
- [ ] 2.3 `ensure_schema` bootstraps constraints + indexes at startup; fail-fast on drift

## 3. Cypher templates
- [ ] 3.1 `cortex-graph/cypher/` with one `.cypher` template per (label × incoming edge) pattern
- [ ] 3.2 Parametrized `UNWIND $rows AS row MERGE ... SET tc += row.props` shapes
- [ ] 3.3 Template loader at startup (compile-checked existence)

## 4. Event-to-graph mapper
- [ ] 4.1 `fn map(&EnrichedEvent) -> GraphPatch` with exhaustive match on `Kind`
- [ ] 4.2 Natural-key computation (Artifact: `repo|path|content_hash`; ULIDs for Decision/Analysis/LawViolation)
- [ ] 4.3 Patch coalescer dedups nodes/edges within a micro-batch

## 5. Worker loop
- [ ] 5.1 Consume enriched events; batch 256 graph-patch entries / 500 ms flush
- [ ] 5.2 Run coalesced patch as single Cypher transaction
- [ ] 5.3 Out-of-order handling: buffer ≤30 s for missing Turn; fabricate `Orphan:true` Turn on timeout
- [ ] 5.4 Failure routing: constraint violation → dead-letter; transient 5xx → retry + consumer pause

## 6. Observability
- [ ] 6.1 Counters + histograms per spec 07 §Observability
- [ ] 6.2 Per-batch span: nodes upserted, edges upserted, dedup hits, tx latency

## 7. Tail (mandatory)
- [ ] 7.1 Update `docs/specs/07-graph-writer.md` status flag to 🟢 + index row
- [ ] 7.2 Integration tests: 10 000-event synthetic stream → correct counts; idempotent replay; coalescer correctness; out-of-order resolution; constraint violation dead-letter; 5xx soak recovery; both Bolt + HTTP transports
- [ ] 7.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
