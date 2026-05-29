# Proposal: phase20_retrieval-relevance-recovery

Source: live audit on 2026-05-27 of all 29 MCP tools against the Cortex production stack.

## Why

The retrieval surface ships green for every MCP tool (every endpoint returns
2xx), but the **content the agent gets back is mostly empty or off-target**.
Live audit numbers from `/v1/status.coverage` on 2026-05-27:

- **Vectorizer**: 565/567 collections empty (99.6% missing). Restart-induced
  loss with no automatic re-ingest; similarity search returns the same
  placeholder hit (`symbol=ToolCall text="" score≈0.05`) for every query.
- **Meili**: 402/567 indexes missing (71% missing). Per-repo `topic_cards`
  index does not exist at all → `cortex_topic_search` always empty.
- **Consolidations**: 41 docs total, 32 stamped manually
  (`model: manual-operator-...`). Consolidator-nightly cron exists but
  produces few automatic rollups — almost all real signal in this corpus
  came from an operator running the CLI by hand.
- **Decisions**: every ADR is `status: proposed` (8 in `cortex_decisions`).
  `decision_search?status=accepted` returns empty. Promotion workflow silent.
- **Laws**: `law_violations?law_id=LAW-CORTEX-001` empty; without the
  filter the index has 819 rows. `law_id` is in the doc body but not in
  Meili's `filterableAttributes`.
- **Graph (Nexus 2.2.0)**: most nodes carry **zero properties**
  (`keys(n) = []`). Only `Session`/`Turn`/`Decision`/`Memory`/`ToolCall`/`Repo`
  have non-empty keys — every other label (Topic, Knowledge, Symbol,
  Concept, Spec, Tool, …) is a bag of `_nexus_id`s.
- **Topic cards**: never seeded. No `cortex-<repo>-topic_cards` index exists.
- **Feedback signals**: `pre_thinking_feedback` table empty.
- **Consolidation cost telemetry**: every doc has `ts=0`,
  `cost_cents=null`, `prompt_tokens=null`. `consolidation_costs` returns
  zero buckets.
- **Active work**: `cortex_active_work` returns empty even with phase19
  in flight.

This proposal is the load-bearing fix for "the data so far doesn't add up
to anything actually relevant" (operator quote, 2026-05-27). The retrieval
contracts are right; the **pipelines that fill them stopped half-way**.

## What Changes

### Data plane — full backfill

- Bring Vectorizer back from 2/567 → ≥95% coverage. Audit the re-ingest
  trigger that was supposed to re-seed after the restart on 2026-05-27;
  fix it or run a one-shot bootstrap. Document the exact command in
  `docs/runbooks/vectorizer-reseed.md`.
- Backfill Meili per-repo indexes for the 402 missing pairs. Reuse the
  classifier-worker + fulltext-worker chain.
- Run the consolidator across every repo with ≥5 captured sessions.

### Graph writer — stamp node properties

- For every node label currently emitting `keys(n) = []`, add the
  property projection in the writer. Minimum per label: `id`, `repo`,
  `kind`, `ts`, plus the label-specific key (`path` for Artifact/Spec,
  `name` for Symbol/Concept/Tool, etc).
- Use Nexus 2.1's reserved `_id` slot (per ADR-004) as the canonical
  external identity — `MATCH (n {id: $id})` must resolve in O(1) hash
  lookup for every label.
- Validate by re-running `cortex_graph_query` neighbors against three
  seeds per label family and confirming non-empty `n.id` in every hit.

### Topic cards — wire the writer end-to-end

- Confirm the topic-card producer is actually firing.
- Provision the per-repo Meili index with the canonical
  filterable/sortable/searchable schema; seed it with the existing
  `topics` taxonomy.
- Acceptance: `cortex_topic_search?topic_prefix=tool:claude-code` returns
  ≥1 card per repo.

### Consolidator nightly — verify the daemon actually fires

- Audit `cron_jobs.retention.consolidator_nightly` runs over the last
  7 days. If no run was triggered (or the daemon is producing zero
  envelopes), trace from cron scheduler → producer → publisher.
- Acceptance: at least one auto-generated `cons-ses-...` consolidation
  per active repo per week.

### Consolidation cost telemetry projection

- Project per-consolidation `cost_cents`, `prompt_tokens`,
  `completion_tokens`, `model_name`, `ts` (envelope occurred_at, not
  zero) onto the Meili doc inside `apply_extensions(Kind::Consolidation)`.
- Acceptance: `cortex_consolidation_costs?group_by=["model","grain"]`
  returns non-empty buckets with real `total_cents` values.

### Consolidation lineage — extend extractor

- Current `extract_sessions`/`extract_files`/`extract_decisions` reads
  only the `topics` array. Many docs (especially manual ones) embed
  lineage in nested `references` JSON or in body markdown as
  `[file](path)` links. Add a second extractor pass over `body` /
  `summary_markdown` / `references`.
- Acceptance: `cortex_consolidation_lineage` against
  `cons-ses-278bab11ad68aa5756df653d` returns non-empty `decisions`
  + `files`.

### Filterable attributes — finish the schema

- `cortex_law_violations?law_id=LAW-CORTEX-001` empty because `law_id`
  is in the doc body but not in `filterableAttributes`. Add `law_id`,
  `severity`, `session_id` to the per-repo `<slug>-governance` schema
  and re-index.
- Confirm `decision_status` exists on every `cortex-<slug>-decisions`
  index (added in phase19 §1.4 — verify uniform coverage).

### Decision promotion workflow

- All 8 ADRs in `cortex_decisions` are `status: proposed`. Either
  (a) gate promotion behind a CI rule that flips an ADR when it lands
  a feature commit referencing it, OR (b) document the manual
  promotion path. Surface in the dashboard which ADRs are stuck.

### Fusion — drop placeholder vector hits

- Every `cortex_query` response includes hits like
  `{source: "vector", symbol: "ToolCall", text: "", score: 0.05}`.
  Empty-text hits contribute nothing but rank pollution. Drop hits
  with `text.is_empty()` before RRF fusion.
- Acceptance: top-3 `cortex_query` results carry non-empty `text`
  ≥100 chars.

### Pre-thinking feedback loop

- `cortex_feedback_signals` reads from a SQLite that nothing populates.
  Add an MCP-side `cortex_feedback_record` tool (intent, query_id,
  helpful: bool, note: string) and call it from the Claude Code
  plugin's post-thinking hook.

### Active work surfacing

- `cortex_active_work` returns empty even with phase19 in flight.
  Audit the tasks_loader against `.rulebook/tasks/*/`.

### Graph lane budget — stop dropping every query

- `query_explain` shows `graph_ms=101 error="budget exceeded"` on
  almost every query. The graph lane never lands a hit inside the
  500ms budget because the seed lookup is unindexed for most labels.
  Fix the unindexed property lookup (covered above) or skip the lane
  when the query has no graph-bearing scope.

### Phase19 tail — known small bugs

- `cortex_consolidations_by_entity?entity.kind=decision_id` returns
  502 `Index cortex_consolidations not found`. Fallback doesn't scope
  per-repo when `entity.kind != "repo"`. Route through the per-repo
  cascade like `consolidations_search` does.
- `cortex_similar_sessions` always returns empty even with
  `confidence_floor=0.1`. The `cortex-<repo>-consolidations`
  Vectorizer collection has 0 vectors today (consequence of the
  Vectorizer-empty fact above; re-validate once that is fixed).

## Impact

- Affected specs: `02-vectorizer-layout`, `07-graph-indexer`,
  `08-fulltext-indexer`, `11-query-api`, `12-pre-thinking`,
  `19-retention-sweep`, `22-fine-grained-search`.
- Affected code: `cortex-api/src/search/**`, `cortex-api/src/orchestrator.rs`,
  `cortex-workers/src/{graph,fulltext,classifier_worker,consolidator}/**`,
  `cortex-mcp-server/src/tools.rs`, `cortex-api/src/tasks_loader.rs`.
- Breaking change: NO (additive — fills holes in existing contracts).
- User benefit: the agent actually gets relevant context back when it
  queries Cortex. Today most lanes return empty or noise.

## Success criteria (top-of-funnel)

Run all 29 MCP tools against a non-trivial repo and report:

1. `cortex_query` returns ≥5 non-empty snippets per query, with the
   top-1 relevant on ≥7/10 manual-eval queries.
2. Vectorizer coverage ≥95% per `/v1/status.coverage`.
3. Meili coverage ≥95% per `/v1/status.coverage`.
4. `cortex_topic_search` returns ≥1 card for at least 5 prefixes.
5. `cortex_consolidations_recent` shows ≥3 auto-generated docs
   (`model != manual-operator-...`) over the last 14 days.
6. `cortex_consolidation_costs` returns non-empty buckets.
7. `cortex_consolidation_lineage` returns non-empty
   `decisions`/`files`/`source_session_ids` for ≥80% of recent docs.
8. `cortex_graph_query?mode=neighbors` returns non-empty `nodes` with
   non-empty `n.id` for every label family.
9. `cortex_law_violations?law_id=<id>` returns the matching subset.
10. `cortex_active_work` reflects the currently active task on disk.
