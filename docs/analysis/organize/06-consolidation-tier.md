# 06 — Consolidation tier (curated memory)

> **Trigger:** *"fazer um parte de consolidados, o que está na memória sendo
> tratado e sumarizado para virar uma memória consolidada, limpando os lixos
> e assim limpando a memória normal, assim os dados vão ser mais relevantes e
> curados do que temos hoje"*
>
> **Sits on top of:** [05-implementation-plan.md](./05-implementation-plan.md)
> (phase 11i). Implementation tracked under
> [`.rulebook/tasks/phase11j_consolidation_tier/`](../../../.rulebook/tasks/phase11j_consolidation_tier/).

## 1. Why a consolidation tier

Phase 11i ingests ~2.4 M raw envelopes (turns + tool_calls + agent_calls).
That's recall. But:

- **90 % of those records are noise** — half-finished tool outputs,
  retry loops, abandoned approaches, attachment hooks the user never
  sees.
- **Embedding 2.4 M raw turns is wasteful** — most never surface in a
  pre-thinking bundle, but they all consume hot-tier vector storage and
  Meili index space.
- **The 32 KiB pre-thinking budget is the real bottleneck** — the agent
  reads top-N hits per section. If those hits are raw fragmented turns,
  the bundle wastes bytes on context-switching boilerplate ("then I ran
  `ls`, then I read foo.rs, then I ran `cat …`") instead of the
  *takeaway* ("we decided to use Meili because Lexum's filter grammar
  is not standard SQL").

Consolidation is the answer: a curated, summarised, evergreen layer
that sits between raw events and the agent. Raw stays as evidence
(audit trail, deep-dive on demand). Consolidated is what the
pre-thinking bundle prefers to surface.

This is the human-memory analogue: episodic → semantic. The brain
doesn't replay every second of every meeting; it stores the
takeaway and the trigger to recall the episode.

## 2. The three consolidation grains

| Grain | What it consolidates | Cardinality | Producer trigger |
|---|---|---:|---|
| `session` | every Turn / ToolCall in one `session_id` | 1-per-session | Stop hook + nightly back-fill |
| `topic` | turns that cluster around a topic across sessions (e.g. "HNSW tuning") | ~50-200 per repo | nightly clustering pass |
| `decision-trace` | the chain of turns / artifacts that led to a single `Kind::Decision` / `Kind::Learning` | 1-per-decision | on Decision emit |

**Session consolidation** answers *"what did this session accomplish?"*
**Topic consolidation** answers *"everything we ever learned about X."*
**Decision-trace consolidation** answers *"how did we arrive at this ADR?"*

All three produce the same Envelope shape (`Kind::Consolidation`,
defined in §4) but differ in scope, source set, and trigger.

## 3. What counts as "lixo" (noise) — pruning rules

Mapped at ingest (phase 11i mapper) and consolidator (phase 11j):

| Class | Action |
|---|---|
| `attachment.type ∈ {file-history-snapshot, queue-operation, last-prompt}` | dropped at mapper — never enters Cortex |
| `attachment.type ∈ {hook_success, hook_additional_context, deferred_tools_delta, skill_listing}` | folded into parent ToolCall metadata or dropped if no parent |
| `tool_call.outcome=error` followed by retry with same input | demoted to cold tier; consolidation captures "we tried X, it failed because Y" |
| Duplicated `tool_call` (same tool + input within 5 s) | first kept, rest deduped |
| Tool outputs > 8 KiB (e.g. `cargo build` full log) | head + tail + hash; full body lives in CAS |
| Sessions with `turn_count < 3` and no Decision / Learning | dropped after 7 days unless flagged |
| PII / secret leaks caught by redactor | hard-purged from raw + consolidation |

Everything else is preserved.

## 4. New `Kind::Consolidation` schema

In `crates/cortex-core/src/events.rs`:

```rust
pub enum Kind {
    // existing variants…
    Consolidation,
}

pub struct ConsolidationPayload {
    pub consolidation_id: String,        // ULID
    pub grain: ConsolidationGrain,       // Session | Topic | DecisionTrace
    pub scope: ConsolidationScope,       // session_id / topic / decision_id
    pub title: String,                   // 80-char one-liner
    pub summary_markdown: String,        // 200-2000 chars curated body
    pub takeaways: Vec<String>,          // bullet "lessons learned"
    pub source_event_ids: Vec<Ulid>,     // every raw event consolidated
    pub source_event_count: u32,         // total (may be > vec len if clipped)
    pub model: String,                   // claude-haiku-4-5 / claude-opus-4-7
    pub depth: ConsolidationDepth,       // Shallow (haiku) | Deep (opus)
    pub outcome_distribution: HashMap<String, u32>, // success/error/blocked counts
    pub temporal_span: TimeSpan,         // {start, end}
    pub repos: Vec<String>,              // repo slugs touched
    pub tags: Vec<String>,
}

pub enum ConsolidationGrain { Session, Topic, DecisionTrace }
pub enum ConsolidationDepth { Shallow, Deep }
pub struct ConsolidationScope { /* discriminated by grain */ }
pub struct TimeSpan { start: DateTime<Utc>, end: DateTime<Utc> }
```

**Routing:** `Kind::Consolidation` → family `consolidations` (new),
collection `cortex-{slug}-consolidations` per repo + global
`cortex_consolidations`. Settings v3 (next bump after 11i §3.3 v2).

## 5. Producer pipeline

```
                     ┌─────────────────────────┐
                     │ cortex-consolidator      │  (new crate)
                     │   crates/cortex-         │
                     │   consolidator/           │
                     └────────────┬─────────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
   ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
   │ session producer │  │ topic producer   │  │ decision-trace   │
   │  (Stop hook +    │  │  (nightly cron + │  │ producer         │
   │   nightly batch) │  │   HDBSCAN on     │  │  (on Decision    │
   │                  │  │   turn vectors)  │  │   emit)          │
   └────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
            │                     │                     │
            └─────────────────────┼─────────────────────┘
                                  │
                                  ▼
                  ┌──────────────────────────────┐
                  │ summariser                   │
                  │  - default: Haiku 4.5        │
                  │  - --deep: Opus 4.7          │
                  │  - prompt template per grain │
                  └──────────────┬───────────────┘
                                 │
                                 ▼
                       Envelope {
                         kind: Consolidation,
                         payload: ConsolidationPayload,
                         parent_event_id: <decision_id when grain=DecisionTrace>,
                       }
                                 │
                                 ▼
                  cortex.events.bootstrap (Synap)
                                 │
                                 ▼
                       (existing 11i pipeline)
```

**Cost guardrails:**

- Haiku 4.5 input cap per consolidation: 32 KiB raw events → ~8 K input
  tokens. Output cap: 2 KiB summary → ~512 tokens. Cost ≈ $0.0008
  per session consolidation.
- Opus only fires when `grain=DecisionTrace` OR user types
  `/cortex consolidate --deep`. Cost ≈ $0.05 per deep call.
- Nightly batch processes only sessions with `last_consolidated_at`
  older than the latest event in the session — idempotent.

## 6. Pruning raw layer

After a consolidation lands and references a raw event:

| Age of raw | Hot tier (FP32) | Warm tier (PQ) | Cold tier (binary) | Meili |
|---|---|---|---|---|
| 0-7 d | ✓ | — | — | full |
| 7-90 d | — | ✓ | — | full |
| 90-365 d | — | — | ✓ (1-bit) | reduced fields |
| > 365 d | — | — | — | dropped |

Source-of-truth for the raw payload always lives in the Parquet
archive (`<CORTEX_ARCHIVE_ROOT>/events/`) — pruning the indexes
never destroys evidence. Re-hydrating is `cortex-bootstrap
--repo <slug> --since <date>`.

**Hard purge** (irrecoverable) only for:
- redactor caught a secret post-ingest
- user typed `/cortex forget <event_id>` (with confirmation)
- `outcome=blocked_by_law` after 30-day grace

## 7. Pre-thinking surfacing

Phase 11i §4 added a "Past sessions" section. Phase 11j replaces it
with **"Consolidated context"** when consolidations exist:

```
## Consolidated context (3)
1. session/01KQH… · 2026-04-30 · Cortex query empty results — root cause stale daemon · ✓ resolved
2. topic/hnsw-tuning · last touched 2026-04-22 · 14 sessions · ✓ stable since v3.0.0
3. decision-trace/D-007 · "use Meili over Lexum" · ✓ active · superseded none
```

Each line is ~120 bytes. Three lines = 360 bytes. Compare to the
raw "Past sessions" section which renders ~400 bytes per line and
dumps three full session previews. Same byte budget, ~3 × the
information density.

When the agent needs the underlying raw events, the consolidation
carries `source_event_ids` — one extra `cortex_query` lookup
fetches them on demand.

## 8. Trust + provenance

Consolidations are agent-authored summaries. The agent reading them
must trust their accuracy or it contaminates downstream reasoning.

**Guardrails:**

1. **Source IDs are mandatory.** Every consolidation lists every raw
   event it summarised. The agent can verify any claim by fetching
   the source.
2. **Model + depth visible.** The bundle line carries
   `(haiku|opus, shallow|deep)` so the agent weighs trust accordingly.
3. **Re-consolidation on change.** When a session gets new events
   (live tail), the existing consolidation is invalidated and
   re-queued.
4. **Eval gate.** A new IT `consolidation_fidelity_it` samples 50
   raw → consolidation pairs and asserts every consolidation
   `takeaways[]` entry is supported by ≥ 1 source event id.
   Threshold: ≥ 90 % supported on Haiku, ≥ 98 % on Opus.

Without these, consolidations risk hallucinating "we decided X" when
no such decision was made.

## 9. Build sequence

Six items, sit on top of phase 11i §3 (relevance axes — the consolidator
wants `outcome` and `model` filters to do its job).

1. New `Kind::Consolidation` + payload (cortex-core)
2. New crate `cortex-consolidator` (producer pipeline)
3. Family / collection / Meili routing (workers + storage)
4. Pre-thinking renderer changes (consolidated context section)
5. Pruning daemon (cron job + reindex pass)
6. Fidelity IT + cost telemetry + tail (docs / tests / verify)

Full task tree in
[`.rulebook/tasks/phase11j_consolidation_tier/tasks.md`](../../../.rulebook/tasks/phase11j_consolidation_tier/tasks.md).
SHALL/Given-When-Then spec in
[`.rulebook/tasks/phase11j_consolidation_tier/specs/`](../../../.rulebook/tasks/phase11j_consolidation_tier/specs/).
