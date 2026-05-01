# Proposal: phase11j_consolidation_tier

Source: [`docs/analysis/organize/06-consolidation-tier.md`](../../../docs/analysis/organize/06-consolidation-tier.md)

## Why

Phase 11i ingests ~2.4 M raw envelopes from `~/.claude/projects/`.
That's recall, not relevance. ~90 % of those records are noise —
half-finished tool outputs, retry loops, abandoned approaches,
attachment hooks the user never sees. The 32 KiB pre-thinking
budget is the real bottleneck: with raw turns, the bundle wastes
bytes on context-switching boilerplate ("then I ran `ls`, then I
read foo.rs, …") instead of the *takeaway* ("we decided to use
Meili because Lexum's filter grammar is not standard SQL").

Consolidation is the human-memory analogue: episodic → semantic.
A curated, summarised, evergreen layer between raw events and the
agent. Raw stays as evidence (audit trail, deep-dive on demand);
consolidated is what the pre-thinking bundle prefers to surface.

Three concrete wins:

1. **Information density per byte** — three consolidation lines
   carry the gist of three full sessions in ~360 bytes vs. raw
   "Past sessions" rendering ~1.2 KiB for the same coverage.
2. **Storage cost** — raw turns demote to cold tier (binary 1-bit)
   after 90 days when a consolidation references them; full payload
   stays in the Parquet archive as evidence, never lost.
3. **Trust signal** — consolidations carry `source_event_ids`,
   model, depth, outcome distribution. The agent can verify any
   claim by fetching sources; eval gate catches hallucinated
   takeaways before they contaminate downstream reasoning.

Depends on `phase11i_claude_archive_indexer_and_relevance` §3
(the consolidator wants `outcome` and `model` filters to cluster
intelligently).

## What Changes

### §1 — `Kind::Consolidation` + payload

New variant in
[`crates/cortex-core/src/events.rs`](../../../crates/cortex-core/src/events.rs).
`ConsolidationPayload`:
- `consolidation_id` (ULID), `grain` (Session | Topic | DecisionTrace)
- `scope` (sessionId / topic / decision_id discriminated by grain)
- `title` (≤ 80 chars), `summary_markdown` (200-2 000 chars)
- `takeaways: Vec<String>` (bullet "lessons learned")
- `source_event_ids: Vec<Ulid>` + `source_event_count` (clipped if huge)
- `model`, `depth` (Shallow | Deep), `outcome_distribution`,
  `temporal_span`, `repos`, `tags`

### §2 — `cortex-consolidator` crate

New sibling crate at `crates/cortex-consolidator/`. Three producer
modes mapped to grains:

- **Session producer** — fires on Stop hook + nightly back-fill;
  consolidates every Turn / ToolCall in one `session_id` into a
  single envelope.
- **Topic producer** — nightly cron; HDBSCAN over turn vectors per
  repo; one consolidation per cluster ≥ 3 sessions.
- **Decision-trace producer** — fires when a `Kind::Decision` lands;
  walks `parent_event_id` chain up to N hops; consolidates the path.

Summariser layer:
- Default: Haiku 4.5 (cost ~$0.0008/session)
- `--deep` flag → Opus 4.7 (cost ~$0.05/call); auto-triggered for
  decision-trace grain or `outcome=success+high-impact` sessions
- Prompt template per grain in `crates/cortex-consolidator/templates/`

### §3 — Family / collection / Meili routing

- New family `consolidations` in
  [`crates/cortex-workers/src/fulltext/routing.rs:213`](../../../crates/cortex-workers/src/fulltext/routing.rs#L213)
- Per-repo collection `cortex-{slug}-consolidations` + global
  Meili index `cortex_consolidations`
- Settings v3 (next bump after 11i §3.3 v2): add `grain`, `depth`,
  `model` to `filterableAttributes`
- Vectorizer payload carries grain + depth + outcome_distribution

### §4 — Pre-thinking renderer

Replace the "Past sessions" section (added in 11i §4.1) with
"Consolidated context" when consolidations exist for the query
scope. Format: one line per consolidation —
`grain/id · date · title · ✓|✗|⚠ outcome`. Top-3 by similarity.
Falls back to raw "Past sessions" only when zero consolidations
match.

### §5 — Pruning daemon

New cron job in `cortex-claude-archive` (extends 11i §5):
- Walk consolidations; for each `source_event_id`, check age:
  - 0-7 d: hot tier (FP32) — keep
  - 7-90 d: warm tier (PQ) — demote
  - 90-365 d: cold tier (binary 1-bit) — demote, reduce Meili fields
  - > 365 d: drop from indexes (Parquet archive untouched)
- Hard purge only on: redactor post-catch, `/cortex forget` user
  command, `outcome=blocked_by_law` after 30-day grace
- IT asserts no source_event referenced by an active consolidation
  is dropped before its consolidation expires

### §6 — Fidelity IT + cost telemetry + tail

- `consolidation_fidelity_it` samples 50 raw → consolidation pairs;
  asserts every `takeaways[]` entry has ≥ 1 supporting
  `source_event_id` (LLM-as-judge with Haiku 4.5 grading;
  threshold ≥ 90 % shallow / ≥ 98 % deep)
- Cost telemetry: `cortex-consolidator` emits per-grain
  $/consolidation + total monthly burn metric; surfaces in
  `/v1/health/coverage`
- Mandatory tail (docs + tests + verify)

## Impact

- **Affected specs:** `01` (event schema — new Kind), `02` (storage
  layout — new family + global index), `08` (Meili settings v2→v3),
  `11` (query API — consolidation lane in strategies),
  `12` (pre-thinking — consolidated context section), `16`
  (dashboard — Consolidations view).
- **Affected code:**
  - **New:** `crates/cortex-core/src/events.rs` (Kind variant +
    payload), `crates/cortex-consolidator/`,
    `crates/cortex-cli/src/bin/cortex-consolidator.rs`,
    `crates/cortex-claude-archive/src/pruner.rs`,
    `crates/cortex-api/tests/consolidation_*_it.rs` (3 ITs).
  - **Modified:** `crates/cortex-workers/src/fulltext/routing.rs`
    (FAMILIES + per-Kind), `crates/cortex-workers/src/fulltext/settings.rs`
    (v3), `crates/cortex-workers/src/embedder/routing.rs`,
    `crates/cortex-storage/src/collections.rs` (+ consolidation FP32),
    `crates/cortex-storage/src/names.rs` (global index),
    `crates/cortex-api/src/strategies.rs` (consolidations lane in
    `pre_change_context` + `similar_problems`),
    `crates/cortex-api/src/archive_loader.rs` (Consolidation case in
    `envelope_to_hit`), `crates/cortex-pre-thinking/src/render.rs`
    (Consolidated context section), `docker-compose.yml` (consolidator
    service).
- **Breaking:** NO. New Kind is additive; settings v3 is auto-applied
  via existing `--apply-settings-only` flag (ships in 11i §3.3);
  pre-thinking falls back to raw rendering when no consolidations
  exist; pruning never destroys Parquet archive evidence.
- **Storage delta:** Consolidations index ~50 KB / 1 K consolidations
  (tiny). Pruning the raw layer reclaims ~70 % of the 11i corpus
  storage cost over the 90-day demotion window — net storage *drops*
  vs 11i alone.
- **User benefit:** Pre-thinking bundles carry curated summaries
  instead of raw turn fragments. Information density per byte ~3 ×
  higher; agent reasoning grounds on takeaways with source pointers
  instead of replaying fragmented context. Storage cost stays
  bounded as the corpus grows past 365 days.

## Source

[`docs/analysis/organize/06-consolidation-tier.md`](../../../docs/analysis/organize/06-consolidation-tier.md)
— full design: noise classification, three grains, producer
pipeline, cost guardrails, pruning rules, trust + fidelity IT.
Read alongside files 01-05 in the same directory for the broader
"organize" plan.
