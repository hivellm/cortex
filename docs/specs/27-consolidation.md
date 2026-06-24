# Spec 27 — Consolidation Daemon Contract

Status: **active** (phase14a)
Replaces: ad-hoc `cortex-consolidator` CLI behaviour described in spec 19 § Cron.
Authors: phase14a_consolidator-daemon-trait-impls.

The consolidator runs as a long-lived daemon that subscribes to a Synap
trigger stream and dispatches each fired trigger to the matching grain.
This spec pins the trait surface (§1), the dispatch contract (§2), the
trigger wire shape (§3), the cost-telemetry plumbing (§4), and the
operator-visible surfaces (§5).

## §1 Consolidator trait

`crates/cortex-workers/src/consolidator/consolidator_trait.rs` defines:

```rust
#[async_trait]
pub trait Consolidator: EnvelopeProducer {
    fn grain(&self) -> ConsolidationGrain;
    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError>;
}
```

Three impls land under `consolidator/grains/`:

| Grain               | Module                                    | Trigger              | Auto-promoted to Opus? |
|---------------------|-------------------------------------------|----------------------|------------------------|
| `Session`           | `grains/session.rs::SessionGrain`         | `SessionEnd`         | no (Haiku default)     |
| `Topic`             | `grains/topic.rs::TopicGrain`             | `NightlyTopic`       | no (Haiku default)     |
| `DecisionTrace`     | `grains/decision_trace.rs::DecisionTraceGrain` | `DecisionLanded` | yes (Orchestrator)     |

Each grain composes `EnvelopeProducer` so the daemon shares the
`producer_checkpoints` SQLite table all phase13b producers write to.

## §2 Dispatch contract

`consolidator/daemon.rs::ConsolidatorDaemon` runs the loop:

1. `source.next_trigger()` pulls one [`PendingTrigger`] from the queue.
2. The daemon builds a per-run `ConsolidatorCtx::with_ledger(now, shared_ledger)`.
3. Dispatch is a match on the [`Trigger`] discriminator — Session ⇒
   `SessionGrain::on_trigger`, NightlyTopic ⇒ `TopicGrain::on_trigger`,
   DecisionLanded ⇒ `DecisionTraceGrain::on_trigger`.
4. On `Ok(report)` the daemon writes one `producer_checkpoints` row keyed
   on `(producer_name, scope)` — `scope` is the trigger's natural key:
   - `Session`           → `session_id`
   - `NightlyTopic`      → `topic:<repo>`
   - `DecisionLanded`    → `decision:<decision_id>`
5. `source.ack(pending.offset)` is called on both the success AND
   failure paths so a poisoned trigger does not block the queue.
6. The failure path skips the checkpoint write so the supervisor does
   NOT treat a poisoned run as completed on resume.

Concurrency: one grain at a time (sequential await). Consolidation is
not throughput-sensitive and the cost ledger + checkpoint contract are
easier to reason about when runs are serial.

## §3 Trigger wire shape

The Synap stream `cortex.consolidator.triggers` carries one JSON envelope
per fired trigger. The bin parses each event via `parse_trigger_event`
in `bin/cortex-consolidator.rs`:

```json
{ "kind": "session_end",       "session_id": "01HXSESS00000000000000000A" }
{ "kind": "nightly_topic",     "repo": "cortex" }
{ "kind": "decision_landed",   "decision_id": "DEC-1", "force_deep": false }
```

Unknown kinds raise a warn-level log and advance the cursor (the
daemon does not infinitely retry a malformed envelope). Missing
required fields raise an error.

### §3.1 Trigger producers (phase24)

Triggers must be published by a producer; until phase24 only the smoke
test wrote to the stream, so the live daemon ran with `dispatched=0`
and the consolidation indexes were never created.

`consolidator/trigger_producer.rs` builds the envelopes. The **decision
-landed** producer is hosted in the classifier worker: it already
enriches every envelope and holds a Synap publisher, so when an enriched
event is `Kind::Decision` it fans out one `decision_landed` trigger via
`decision_landed_trigger(&EnrichedEvent) -> Option<Value>`.

The hook is gated behind `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED`
(default **off**): the decision-trace grain auto-promotes to Opus, so
firing it per decision is opt-in spend. Re-triggering the same decision
is safe — the daemon's `decision:<decision_id>` producer checkpoint
(§2.4) skips a decision already consolidated. Trigger-publish failures
are non-fatal: enrichment + ack proceed regardless.

The **session-end** producer is now also wired in the classifier worker
(phase24 §1.2). A `session_last_seen: Mutex<BTreeMap<String, i64>>`
tracks the last wall-clock ms for each `session_id`. On every enriched
event, `evaluate_idle_sessions` stamps the current session and returns
any session silent for more than `SESSION_IDLE_MS` (30 min). Each
returned session gets a `session_end` trigger via
`session_end_trigger(&session_id)`. The lock is released before the
async publish so no await is held under it.

The **nightly-topic** producer is also live in the classifier worker
(phase24 §1.3). Because the worker holds no Meili client, it cannot
read the current `TopicCardPayload` to feed `TopicTrigger::evaluate`.
Instead a `repo_event_counts: Mutex<BTreeMap<String, u32>>` tracks
per-repo event counts in-process. When the count for a repo reaches
`TRIGGER_EVENTS_THRESHOLD` (8) a `nightly_topic` trigger is published
via `topic_threshold_trigger(repo, None, kind, &|| 1.0, 0)` (the
`card=None` first-emit path always returns `Rewrite`) and the counter
resets. This provides rate-limited topic triggers without a Meili
round-trip on the hot path.

Both the session-end and nightly-topic hooks share the same opt-in flag
(`CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED`, default **off**) and
the same non-fatal publish contract as the decision-landed hook.

The `run-session` / `run-topic` / `nightly` CLI subcommands remain
available as the manual override path.

## §4 Cost telemetry (phase14a §2.4)

`ConsolidatorCtx` owns `Arc<Mutex<CostLedger>>`. Every grain calls
`ctx.record_cost(grain_label, model, cost_cents, prompt_tokens,
completion_tokens)` after the orchestrator run lands. `TopicGrain`
calls it once per cluster.

`CostLedger::record_full` updates per-grain `consolidations`,
`cost_cents`, `input_tokens`, `output_tokens`, and inserts the model
id into a `models_used: BTreeSet<String>` so the dashboard can surface
auto-promotion when DecisionTrace falls back to Haiku.

## §5 Operator surfaces

| Surface                                  | Owner                         | Notes                                                       |
|------------------------------------------|-------------------------------|-------------------------------------------------------------|
| `cortex-consolidator daemon`             | `bin/cortex-consolidator.rs`  | Long-running entry; subscribes to the trigger stream.       |
| `/v1/health/consolidator`                | `cortex-api/src/health/consolidator.rs` | Per-grain `last_run` + `last_status` + counters.            |
| `gui/src/views/Consolidations.tsx`       | dashboard                     | Renders `DaemonHealthPanel` above the consolidation list.   |
| `docker compose service cortex-consolidator` | `docker-compose.yml`          | Healthcheck = `pgrep -f cortex-consolidator`.               |

## §6 Shutdown semantics

`ConsolidatorDaemon::run_forever(shutdown)` selects between
`run_once` and the shutdown future. The in-flight iteration always
finishes before the loop re-checks the signal so producer-checkpoint
writes never tear. The bin wires Ctrl-C via `tokio::signal::ctrl_c`
plus SIGTERM via `tokio::signal::unix::SignalKind::terminate` on
unix; Windows falls back to the Ctrl-C path only.

## §7 MCP surface (phase19 consolidation-first verbs)

Phase19 lands six consolidation-scoped MCP verbs against the
`cortex_consolidations` Meili index that this daemon writes
to. Full wire shape lives in spec 22 "Phase19 — Granular tool
surface § Group B"; pointers from the daemon's perspective:

- `cortex_consolidation_get` — single-doc lookup by `event_id`
  OR producer-stable `consolidation_id`; resolves re-emitted
  envelopes via OR filter
  `event_id="X" OR ext.consolidation.consolidation_id="X"`.
- `cortex_consolidations_recent` — chronological feed sorted
  `occurred_at:desc` with optional `repo`/`grain` filter; grain
  vocab pinned to the producer enum
  (`ConsolidationGrain::{Session, Topic, DecisionTrace}` —
  §1).
- `cortex_consolidations_by_entity` — surfaces consolidations
  referencing a `file`/`function`/`decision_id`/`repo`/`model`
  via filter (`repo`/`model`) or keyword fallback
  (everything else; classifier-extracted entities ship to
  Nexus, not Meili).
- `cortex_consolidations_search` — BM25 over title /
  summary_markdown / topics / repo; reserved for hybrid+RRF
  once `cortex_query` exposes a `kinds=consolidation` scope.
- `cortex_consolidation_lineage` — doc-only projection
  (`source_session_ids` from `topics:session:*`, `decisions`
  from `topics:decision:*` ∪ `DEC-\d{3,}` body regex, `files`
  from `topics:file:*`, `cost.model` from
  `ext.consolidation.model`). Per-consolidation cost +
  source-event-ids require a writer-side projection (§4 ledger
  is in-process per-grain).
- `cortex_consolidations_diff` — `occurred_at:asc` poll cursor
  for "new consolidations since `since_ts`" (the schema lacks
  `accumulated_at`, see deviation note in the handler).

The cost-telemetry tool `cortex_consolidation_costs`
aggregates counts from the same `cortex_consolidations` Meili
index (NOT the in-process `CostLedger` of §4, which is not
persisted and not addressable per-consolidation). Live spend
remains on `/v1/health/coverage`.

Every consolidation-first verb is exercised end-to-end by a
wiremock IT in `crates/cortex-mcp-server/tests/`; cross-tool
ordering is verified by the registry-size assertion in
`crates/cortex-mcp-server/src/server.rs::tests`.
