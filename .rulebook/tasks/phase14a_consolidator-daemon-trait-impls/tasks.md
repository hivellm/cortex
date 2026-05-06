## 1. Consolidator trait
- [ ] 1.1 New trait in `cortex-workers/src/consolidator/trait.rs`: `pub trait Consolidator: EnvelopeProducer { fn grain(&self) -> Grain; async fn on_trigger(&self, trigger: Trigger, ctx: ConsolidatorCtx) -> Result<ConsolidationReport>; }`.
- [ ] 1.2 `Trigger` enum: `SessionEnd { session_id }`, `NightlyTopic { topic_id }`, `DecisionLanded { decision_id }`.
- [ ] 1.3 `ConsolidationReport { grain, trigger, envelopes_emitted, cost_cents, latency_ms, source_event_count }`.
- [ ] 1.4 4 unit tests on the trait shape.

## 2. 3 grain impls
- [ ] 2.1 `impl Consolidator for SessionGrain` — clusters turns by session_id, summarises via the existing prompt template.
- [ ] 2.2 `impl Consolidator for TopicGrain` — nightly clustering of topic_cards.
- [ ] 2.3 `impl Consolidator for DecisionTraceGrain` — runs on DecisionLanded, builds the trace from supersession chain.
- [ ] 2.4 Centralised cost telemetry: every grain calls `ConsolidatorCtx::record_cost(cents, model, prompt_tokens, completion_tokens)`.
- [ ] 2.5 Per-grain IT exercising the trigger → emit path against fixture events.

## 3. Daemon binary
- [ ] 3.1 New `crates/cortex-workers/src/bin/cortex-consolidator.rs` main loop. Subscribes to triggers via Synap consumer group `cortex.consolidator`.
- [ ] 3.2 Dispatches each trigger to the matching grain. Concurrency: one grain at a time (consolidation is not throughput-sensitive).
- [ ] 3.3 Graceful shutdown on SIGTERM: finishes in-flight grain, writes checkpoint, exits 0.
- [ ] 3.4 Add to `docker-compose.yml` as `cortex-consolidator` service with health check.

## 4. Health endpoint + dashboard
- [ ] 4.1 `cortex-api /v1/health/consolidator` returns `{ session_grain: { last_run, last_status }, topic_grain: ..., decision_trace_grain: ... }`.
- [ ] 4.2 Dashboard `Consolidations` view (already shipped in working tree) consumes the new endpoint and shows last-run + last-status per grain.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/15-consolidation.md` + `CHANGELOG.md`.
- [ ] 5.2 Tests: §1.4 + §2.5 × 3 + §3 daemon-shutdown IT + §4.1 endpoint IT.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
- [ ] 5.4 Smoke against running stack: trigger SessionEnd; assert envelope appears in `cortex_consolidations` index within 60s.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
