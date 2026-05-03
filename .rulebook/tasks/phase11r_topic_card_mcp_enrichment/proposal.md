# Proposal: phase11r_topic_card_mcp_enrichment

## Why

Phase11j ships consolidations (Session / Topic / DecisionTrace
grains) — one-shot summaries of N source events, used by
pre-thinking as a denser substitute for raw turns.

That covers retrospective summarisation. It does NOT cover the
living-knowledge contract that 2026-era second-brain systems
(Karpathy LLM Wiki, Obsidian + Claude MCP plugins, Google Memory
Agent pattern) deliver to the model:

1. A topic surface that the LLM rewrites when new evidence lands
   — not a static cluster summary.
2. Contradictions surfaced explicitly when sources diverge
   (e.g. ADR-002 supersedes ADR-001 but a recent law violation
   cites the deprecated rule).
3. Staleness signal so the model knows when to trust the
   synthesis vs. force a fresh roll-up (`synthesis_age_d`,
   `events_since_last_rev`).
4. Drill-down via MCP, not via raw vector hits — the model asks
   `cortex_topic_drill(topic_id, dim="contradictions")` instead
   of receiving 12 chunks and synthesising in-context.

Without this layer Cortex's pre-thinking still returns
fragments-plus-consolidations; the model spends tokens
re-synthesising every turn and cannot detect contradictions or
staleness. Obsidian-pattern systems beat us on perceived
"institutional memory" precisely because their LLM-maintained
wiki is rewritten reactively; Cortex's edge (governance, audit,
multi-tool capture) is not legible to the model unless we
materialise it in a card the model consumes via MCP.

This task is MCP-first. Zero markdown projection on disk; zero
human-vault export. The TopicCard is a structured payload the
model receives via dedicated MCP tools, with optional
pre-thinking injection.

Depends on phase11j §1-§4 (TopicCard cites Consolidations as
evidence; MCP drill-down can fall back to consolidation grain
when a topic card is stale). Does NOT depend on phase11j §5
(pruning) or phase11o (Vectorizer demotion API) — TopicCards
are synthesised content, not pruned raw events.

## What Changes

### §1 — `Kind::TopicCard` + payload

New variant in `crates/cortex-core/src/events.rs` carrying
`TopicCardPayload`:

- `topic_card_id` (deterministic ULID derived from `topic_slug`
  + repo scope hash)
- `topic_slug` (kebab-case, ≤ 80 chars, unique per repo scope)
- `repos: Vec<String>` (scope; usually 1)
- `revision` (monotonic u32, increments on every rewrite)
- `synthesis_markdown` (200-4 000 bytes, LLM-maintained prose)
- `evidence: Vec<EvidenceRef>` — typed refs to Consolidations,
  Decisions, Laws, Turns; each carries `kind`, `id`, `weight`,
  `cited_at_rev`
- `contradictions: Vec<Contradiction>` —
  `{ kind, evidence_a, evidence_b, surfaced_at_rev, status }`
  where `status ∈ {open, reconciled, deprecated}`
- `open_questions: Vec<String>` — ≤ 8 items, ≤ 200 bytes each
- `related_topic_ids: Vec<TopicCardId>` (graph adjacency, ≤ 32)
- `confidence: f32` (0.0-1.0; derived from evidence count + age)
- `last_rev_at` (chrono::DateTime<Utc>)
- `events_since_last_rev: u32` (counter cleared on rewrite)
- `synthesis_model` (Haiku45 / Opus47),
  `synthesis_cost_cents: u32`

Validation cross-field: `evidence.len() ≥ 1`, `revision == 1` ↔
`events_since_last_rev == 0` on first emit, every
`contradictions[*].evidence_*` MUST reference an item in
`evidence`.

### §2 — `cortex-topic-cards` crate

New sibling crate at `crates/cortex-topic-cards/`. Three layers:

**Synthesiser layer.** Reuses `cortex-consolidator::summariser`
(Summariser trait + AnthropicSummariser + cost-budget plumbing)
— zero duplication. `TopicCardSynthesiser::rewrite(card_or_none,
new_evidence, previous_evidence) → ProducedTopicCard`. Haiku 4.5
default; Opus 4.7 escalation when (a) `contradictions.len() ≥ 3`,
or (b) any evidence carries a `Kind::Decision` superseded by
another evidence item, or (c) operator passes `--deep`. Prompt
template at `templates/rewrite.md` with slots: `topic_slug`,
`existing_synthesis`, `existing_revision`, `new_evidence_block`,
`superseded_evidence_block`, `output_contract`. Output JSON:
`{ synthesis_markdown, contradictions, open_questions,
confidence }`. Producer parses + builds the payload, increments
`revision`, resets `events_since_last_rev`, stamps
`last_rev_at = now()`, carries `synthesis_cost_cents` from the
summariser.

**Reactive trigger layer.** `Trigger::evaluate(card, new_event)
→ TriggerDecision` returns `Rewrite | Skip { reason }`.
Heuristics: rewrite when `events_since_last_rev ≥ N`
(default 8), OR `embedding_distance(new_event, card.synthesis)
< 0.30` AND the event carries a `Decision` / `LawViolation` /
high-impact outcome, OR `synthesis_age_d ≥ 14` and any new
evidence lands. Subscribes to Synap `events.classified` topic;
emits to `events.topic_card.rewritten` after each rewrite.
Idempotent per `(topic_slug, repo_scope)` — concurrent rewrites
coalesce via Synap pub/sub lease
(`topic_card.rewrite.lease.{slug}`).

**Contradiction detector.** `ContradictionScanner::scan(evidence)
→ Vec<Contradiction>` surfaces three classes:
`decision_supersession` (two `Kind::Decision` refs where one
supersedes the other; deprecated carries `status = open`),
`law_violation_mismatch` (a `Kind::LawViolation` cites a Law
version different from the latest active version),
`outcome_divergence` (two consolidations with overlapping
temporal_span carry conflicting `outcome_distribution`).
Heuristic only — Opus escalation can override with
reconciliations; the detector never blocks rewrite.

CLI binary `crates/cortex-cli/src/bin/cortex-topic-cards.rs`:
subcommands `rewrite <topic_slug>`, `scan-now`,
`replay --since <ts>`, `nightly --dry-run`. Reads
`ANTHROPIC_API_KEY` / `ANTHROPIC_API_URL` /
`--monthly-cents-cap` identical to `cortex-consolidator`.

### §3 — Family / collection / Meili routing

- New family `topic_cards` in
  `crates/cortex-workers/src/fulltext/routing.rs` (FAMILIES +
  `family_for(Kind::TopicCard)`).
- Embedder routing emits per-repo collection
  `cortex-{slug}-topic_cards`.
- New global Meili index `cortex_topic_cards`; per-repo index
  `cortex-{slug}-topic-cards`.
- New collections in `crates/cortex-storage/src/collections.rs`:
  `cortex.topic_card.fp32` (hot, recall-tuned `m=48 / ef=256`)
  and `cortex.topic_card.pq` (warm, `m=16 / ef=64`).
- Settings v4 → v5 in
  `crates/cortex-workers/settings/settings.v1.json`: add
  `ext.topic_card.{topic_slug, revision, confidence,
  synthesis_age_d, contradictions_count, repos}` to
  `filterableAttributes` + `sortableAttributes`; add
  `synthesis_markdown` to `searchableAttributes`.
- Classifier (`cortex-classifier/src/statics.rs`) gains a
  `Kind::TopicCard` arm pushing `topic_cards` + the topic_slug
  as topics.
- Graph mapper emits a `:TopicCard` node with edges
  `EVIDENCE_OF → :Consolidation | :Decision | :Law | :Turn`
  and `RELATED_TO → :TopicCard` (bidirectional, dedup on write).
- `cortex-bootstrap --apply-settings-only` picks up the new
  index (ALL_INDEXES extended) — same one-shot operator
  workflow phase11j §3.7 established.

### §4 — MCP tools

Five new tools shipped via `crates/cortex-api/src/mcp.rs`:

- `cortex_topic_get(query_or_slug, scope) → TopicCard | null` —
  slug-exact short-circuit; query path runs hybrid search
  (fulltext + vector RRF) over the topic_cards family and
  returns top-1 if confidence ≥ 0.6, else null.
- `cortex_topic_drill(topic_card_id, dimension)` —
  `dimension ∈ {evidence, contradictions, history,
  open_questions, related}`. `evidence` returns the typed list
  hydrated with each item's title + occurred_at; `history`
  returns `Vec<TopicCardRevision>` (id, revision, last_rev_at,
  summary diff hash); `related` returns up to 32 sibling cards
  with outgoing edges.
- `cortex_topic_neighbors(topic_card_id, depth=2)` — runs a
  Nexus Cypher walk over `RELATED_TO` + `EVIDENCE_OF`
  (≤ depth hops), returns the subgraph clipped at 64 nodes.
- `cortex_topic_diff(topic_card_id, since_rev)` — returns
  `{ from_rev, to_rev, synthesis_diff, evidence_added,
  evidence_removed, contradictions_added,
  contradictions_resolved }`.
- `cortex_synthesize(query, scope, force=false)` — operator
  escape hatch; runs the synthesiser ad-hoc (no card persisted
  unless `persist=true`), returns the same payload shape.
  Counts against the monthly budget.

All tools emit audit envelopes (cortex-api `audit::record_call`)
so phase11j's audit lineage extends naturally.

### §5 — Pre-thinking renderer integration

Replace the priority order in
`crates/cortex-pre-thinking/src/render.rs`:

1. Active laws (unchanged)
2. **Topic cards (new)** — top-1 by hybrid score per query intent;
   render the card synthesis (≤ 600 bytes clip), then a compact
   evidence block (top-5 items), then `contradictions` (only
   `status = open`), then a `staleness` line
   (`synthesis_age_d`, `events_since_last_rev`).
3. Consolidations (existing — phase11j §4.2)
4. Decisions
5. Similar turns
6. Past sessions (fallback when zero topic cards + zero
   consolidations match)
7. Snippets (existing)

`FormatOptions` gains `topic_cards_cap: usize = 1` (the
synthesis is dense; > 1 burns the 32 KiB budget). New section
budget `section_caps::TOPIC_CARDS = 1_400` bytes.

When `confidence < 0.6` OR
(`synthesis_age_d > 30` AND `events_since_last_rev > 0`), the
renderer SHALL emit a `stale-topic-card` advisory line and
DOWNGRADE the topic card section to fallback (consolidation
block runs first instead). This avoids feeding the model stale
syntheses without an explicit signal.

### §6 — Tail (docs + tests + verify)

Mandatory tail enforced by rulebook v5.3.0. Spec deltas land in
`specs/topic-card/spec.md` and propagate to specs 01, 02, 08,
11, 12, 16 on archive.

## Impact

- **Affected specs:** 01 (event schema — new Kind), 02 (storage
  layout — new family + global index + collections), 08 (Meili
  settings v4 → v5), 11 (query API — topic_cards lane in
  strategies + 5 new MCP tools), 12 (pre-thinking — section
  reorder + staleness advisory), 16 (dashboard — TopicCards
  view).
- **Affected code:**
  - **New:** `crates/cortex-core/src/events.rs` (Kind variant +
    payload), `crates/cortex-topic-cards/`,
    `crates/cortex-cli/src/bin/cortex-topic-cards.rs`,
    `crates/cortex-api/tests/topic_card_mcp_it.rs`,
    `crates/cortex-pre-thinking/tests/topic_cards_render_it.rs`,
    `crates/cortex-topic-cards/tests/end_to_end_it.rs`,
    `docs/cortex/topic-cards.md`,
    `.rulebook/decisions/006-topic-card-as-living-synthesis-vs-consolidation-as-snapshot.md`.
  - **Modified:**
    `crates/cortex-workers/src/fulltext/routing.rs` (FAMILIES +
    family_for), `crates/cortex-workers/src/embedder/routing.rs`,
    `crates/cortex-workers/settings/settings.v1.json` (v4 → v5),
    `crates/cortex-storage/src/collections.rs` (+2 collections),
    `crates/cortex-storage/src/names.rs` (+ index + collection
    constants, ALL_INDEXES extended),
    `crates/cortex-classifier/src/statics.rs`
    (Kind::TopicCard arm),
    `crates/cortex-workers/src/graph/mapper.rs`
    (`:TopicCard` label + edges),
    `crates/cortex-api/src/strategies.rs` (topic_cards lane),
    `crates/cortex-api/src/archive_loader.rs` (envelope_to_hit
    arm), `crates/cortex-api/src/mcp.rs` (5 new tools),
    `crates/cortex-pre-thinking/src/render.rs` (section reorder
    + advisory),
    `crates/cortex-pre-thinking/src/formatter.rs`
    (TopicCards section), `Cargo.toml` workspace members,
    `docs/specs/11-query-api.md`,
    `docs/specs/12-pre-thinking-injection.md`,
    `docs/specs/16-dashboard.md`, `CHANGELOG.md`.
- **Breaking:** NO. New Kind is additive. Settings v4 → v5
  applied via existing `--apply-settings-only`. Pre-thinking
  falls back to the phase11j ordering when zero topic cards
  match. MCP tools are additive (existing tools untouched).
- **Cost:** TopicCards rewrite ~10× less often than
  consolidations (one card per topic vs. one consolidation per
  session). Estimated steady-state burn for the cortex
  monorepo scope: ~40 rewrites/day × $0.0008 (Haiku) +
  ~3 escalations/day × $0.05 (Opus) ≈ $5/month. Well under
  the $1 000/month guardrail enforced by
  `cortex-consolidator::cost_telemetry` (the same `CostBudget`
  is shared across both producers).
- **Storage delta:** TopicCards index ≈ 200 KB / 1 000 cards.
  Graph adjacency adds ~10 K edges in Nexus
  (`RELATED_TO` + `EVIDENCE_OF`).
- **Latency delta:** Pre-thinking adds one extra hybrid-search
  call (top-1 over `cortex_topic_cards`) on the hot path.
  Budget impact: P50 +3-5 ms, P99 +10-15 ms. Within the
  spec-12 budget (P50 < 50 ms).
- **User benefit:** The model receives a single dense synthesis
  per topic instead of N consolidations + M raw snippets it
  has to reconcile in-context. Contradictions surface
  explicitly so the model never silently averages conflicting
  evidence. Staleness signal lets the model decide whether to
  trust the cache or force a fresh roll-up. MCP drill-down
  replaces "give me 12 chunks" with "give me the open
  contradictions for this topic" — fewer tokens, sharper
  context, auditable trail.

## Source

User analysis dated 2026-05-03, two turns:

1. "por que todos estao usando obsidian.md para melhorar a
   memoria persistente das LLM, e como podemos melhorar o
   cortex"
2. Recalibration: "queremos que o cortex seja usado via
   mcp/plugin para consultas complexas para enriquecer o
   thinking e os resultados pra LLM"

Karpathy LLM Wiki pattern (April 2026) provided the
living-synthesis primitive. Obsidian + Claude MCP plugins
(jacksteamdev/obsidian-mcp-tools, iansinnott/obsidian-claude-code-mcp,
2026) provided the MCP-tool surface inspiration. Cortex
contributes governance + audit + contradictions — the
differentiators that markdown-only second brains cannot match.
