## 1. Crate scaffold
- [x] 1.1 `cortex-graph` crate with `GraphWriter` trait + `GraphPatch` types
- [x] 1.2 Worker binary `cortex-graph-worker` skeleton (consumer wiring pending in §5)
- [x] 1.3 Config via env (`CORTEX_GRAPH_*`)

## 2. Nexus client
- [x] 2.1 RPC transport via `nexus-graph-sdk` (shared `Arc<dyn Transport>` instead of `deadpool`) + retry policy (3 attempts, exp backoff 100/400/1600 ms)
- [x] 2.2 HTTP fallback behind `CORTEX_GRAPH_TRANSPORT=http`
- [x] 2.3 `ensure_schema` bootstraps constraints + indexes at startup; fail-fast on drift

## 3. Cypher templates
- [x] 3.1 `cortex-graph/cypher/` with one `.cypher` template per (label × incoming edge) pattern
- [x] 3.2 Parametrized `UNWIND $rows AS row MERGE ... SET tc += row.props` shapes
- [x] 3.3 Template loader at startup (compile-checked existence) — `REQUIRED_TEMPLATES` + `ensure_required` fail-fast in `main.rs`
- [x] 3.4 Switch `LiveNexusClient::run_write_tx` from inline-Cypher generation to template registry

## 4. Event-to-graph mapper
- [x] 4.1 `fn map_event_to_patch(&EnrichedEvent) -> GraphPatch` with exhaustive match on `Kind`
- [x] 4.2 Natural-key computation (Artifact: `repo|path|content_hash`; event_id for Turn/ToolCall/Memory/Decision/Analysis/LawViolation)
- [x] 4.3 Patch coalescer dedups nodes/edges within a micro-batch
- [x] 4.4 Per-kind payload expansion: `TOUCHED` (ToolCall→Artifact per file), `LINKED_TO` (Turn→Decision, role=status via `parent_event_id`), `OF` (LawViolation→Law), `SUPERSEDES` (Decision→Decision), Turn-anchored `HAS_TOOL_CALL` (with Session fallback for orphans)
- [ ] 4.5 `OBSERVED_IN` (LawViolation → Turn|ToolCall) — blocked by upstream schema: `LawViolationPayload.observed_event_id` carries no kind discriminator, so the writer cannot choose the target label without phantom-node risk via Cypher `MERGE`. Unblocks once the payload gains `observed_event_kind`.

## 5. Worker loop
- [x] 5.1 Consume `cortex.events.enriched` from Synap; batch 256 graph-patch entries / 500 ms flush (`SynapConsumer`/`SynapPublisher` traits + `LiveSynapConsumer`/`MemorySynapConsumer`, `OffsetTracker`)
- [x] 5.2 Run coalesced patch as single Cypher transaction and publish report on `cortex.events.graphed` (via `GraphWriter::write_patches` with orphan injection)
- [x] 5.3 Out-of-order handling: `OutOfOrderBuffer` holds `tool_call`/`agent_call` events whose parent Turn is unseen, sweep emits orphan-Turn nodes (`orphan: true`) past `out_of_order_buffer_secs`
- [x] 5.4 Failure routing: `ConstraintViolation` → `cortex.events.invalid` per event; `TransientError` → backpressure gauge + soak-pause; deserialize failures route to invalid stream

## 6. Observability
- [x] 6.1 Counters + histograms per spec 07 §Observability (`Metrics` registry: nodes/edges upserted, dedup hits, tx latency, tx size)
- [ ] 6.2 Per-batch tracing span wired through the worker loop (nodes upserted, edges upserted, dedup hits, tx latency)

## 7. Tail (mandatory)
- [ ] 7.1 Update or create documentation covering the implementation — flip `docs/specs/07-graph-writer.md` status to 🟢 + index row
- [ ] 7.2 Write tests covering the new behavior — integration tests: 10 000-event synthetic stream → correct counts; idempotent replay; coalescer correctness; out-of-order resolution; constraint violation dead-letter; 5xx soak recovery; both RPC + HTTP transports
- [ ] 7.3 Run tests and confirm they pass — `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
