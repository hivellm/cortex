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
