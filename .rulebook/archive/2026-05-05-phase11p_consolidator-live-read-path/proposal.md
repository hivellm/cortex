# Proposal: phase11p_consolidator-live-read-path

## Why

Phase11j shipped the consolidator producers (`crates/cortex-workers/src/consolidator/producer/{session,topic,decision_trace}.rs`) and phase11o shipped the pruner end-to-end, but the pipeline never closes in production:

- `crates/cortex-workers/src/bin/cortex-consolidator.rs:192,204,220,239` — every operational subcommand (`run-session`, `run-topic`, `run-decision`, `nightly --dry-run=false`) returns `status: pending §3 routing wiring (live envelope read path)`. The producers exist but nothing populates `SessionInput` / `TopicCluster` / `DecisionTraceInput` from live Synap + Vectorizer + Nexus.
- `crates/cortex-workers/src/retention/scheduler.rs:226` — the cron seed `retention.memory_consolidate` is `enabled: false`, so the auto-memory consolidator (Claude Code memory dir; phase9h, fully implemented in `crates/cortex-cli/src/ops/memory_consolidate.rs`) never fires either.
- No cron seed exists for the envelope consolidator (`cortex-consolidator nightly`); even with the read path wired, nightly would not run.

Result: zero rows ever land in `cortex_consolidations`. The pruner's nightly sweep at 03:00 (phase11o §2.5) walks an empty index, demotes nothing, purges nothing — raw envelopes in Synap streams + Vectorizer collections + Meili indices grow without bound. The user observes "memory keeps growing" because the only mechanism that bounds it (consolidation → demote/purge of source events) is structurally inert.

## What Changes

1. **Live read path for the envelope consolidator.** Three sources land under `crates/cortex-workers/src/consolidator/source/`:
   - `session.rs` — `LiveSessionSource` reads a session's envelopes from Synap (`union_read_sessions` exists in `crates/cortex-storage/src/metadata.rs:978`; query by `session_id`, materialise `Vec<Envelope>` ordered by `occurred_at`).
   - `topic.rs` — `LiveTopicSource` queries Vectorizer for one-line digests + their embeddings inside a per-repo time window, runs HDBSCAN (workspace `hdbscan` dep already pinned by phase11j §2.1), emits `Vec<TopicCluster>` with `MIN_CLUSTER_SIZE = 3`.
   - `decision_trace.rs` — `LiveDecisionTraceSource` walks `parent_event_id` from a `Kind::Decision` envelope through Synap up to `MAX_HOPS = 16`, returns `DecisionTraceInput`.
2. **Bin wiring.** `cortex-consolidator` binary's four subcommands swap their `pending` stubs for `LiveSessionSource::fetch` → `Orchestrator::run_session`, etc. `nightly` enumerates sessions closed in the last 24 h, runs sessions first, then topic clusters per repo, then any new decisions.
3. **Cron seeding.** New seed `retention.consolidator_nightly` in `default_jobs()` (02:00 — runs before the 03:00 pruner so the pruner has fresh consolidations to act on). Flip `retention.memory_consolidate` from `enabled: false` → `enabled: true`.
4. **IT live in container.** New IT (gated `CORTEX_CONSOLIDATOR_LIVE_IT=1`) seeds 30 envelopes into a real Synap stream, runs `LiveSessionSource::fetch` + `Orchestrator::run_session` against a `CannedSummariser` (no API gate burnt), asserts the row lands in `cortex_consolidations` Meili index with the right grain/scope shape.
5. **Doc + spec deltas.** Spec 12 (pre-thinking) already cites the consolidations lane; spec 19 (retention) needs a one-line confirmation that the producer side now feeds the pruner. Architecture doc §6.0 (tiered storage) adds the cron schedule diagram.

## Impact

- Affected specs: `docs/specs/19-retention.md`, `docs/architecture.md` §6.0
- Affected code: `crates/cortex-workers/src/consolidator/source/{session,topic,decision_trace}.rs` (new), `crates/cortex-workers/src/bin/cortex-consolidator.rs`, `crates/cortex-workers/src/retention/scheduler.rs::default_jobs()`
- Breaking change: NO (additive — no wire changes, no schema bumps; cron flip is operator-visible but matches the documented intent of phase11j)
- User benefit: closes the consolidation→pruner loop so raw event growth is actually bounded by the design that already shipped. Without this task, phase11j+phase11o together produce zero observable effect on disk/RAM growth.

## Source

phase11j §5 was deferred to phase11o (Vectorizer SDK gap); phase11o shipped but the producer side remained on the §3 stub. Conversation 2026-05-04: user surfaced unbounded memory growth despite both phases archived as done.
