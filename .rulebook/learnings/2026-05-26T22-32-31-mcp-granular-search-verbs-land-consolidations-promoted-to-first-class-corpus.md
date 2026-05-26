# MCP granular search verbs land — consolidations promoted to first-class corpus
**Source**: manual
**Date**: 2026-05-26
**Related Task**: phase19_mcp-granular-search
**Tags**: mcp, search, consolidation, meili, sqlite, match-strategy, scope-deviation

Symptom: every retrieval question went through the fused `cortex_query` →
RRF over keyword + vector + graph lanes, projected to a tight set of
`results.*` lists. Callers needing "show me every ToolCall in repo X this
week" or "every consolidation that mentions DEC-0042" had to post-filter
through the pre-thinking byte budget; consolidation envelopes were
reachable only via `cortex_similar_sessions` (vector-only, no metadata
filter, no lineage walk, no diff cursor).

Lesson 1 — **the `cortex_consolidations` Meili index IS the right
backbone for a consolidation-first MCP surface**, but the writer's
projection contract is the gating constraint. The phase19 design
hit four documented projection gaps that landed as `match_strategy`
discriminators on the response rather than schema bumps + reindexes:

  - `cortex_consolidations_search` ships BM25-only because the
    `/v1/search/vector` proxy rejects `query_text` with
    `not_implemented` (server-side embedding lives behind
    `QueryService`, not the search proxy) AND `cortex_query` has no
    `kinds=consolidation` scope. Adding either is a structural
    change. `match_strategy = "bm25"`; reserved `"hybrid_rrf"` flips
    when one lands.
  - `cortex_consolidation_lineage` is doc-only because
    `crates/cortex-workers/src/fulltext/builders.rs::apply_extensions`
    Kind::Consolidation arm only projects `model` +
    `source_event_count` — `source_event_ids` /
    `source_session_ids` / cost telemetry are NOT projected. The
    handler derives the lineage from `topics` (`session:` /
    `file:` / `decision:`) + a `DEC-\d{3,}` body regex.
    `match_strategy = "doc_only"`; reserved `"joined"` flips when
    the writer projects.
  - `cortex_consolidation_costs` aggregates **counts** rather than
    cents/tokens because `grain_costs` is an in-process
    `CostLedger` keyed by grain label — NOT a SQL table, NOT
    addressable per consolidation. Live spend remains on
    `/v1/health/coverage`.
  - `cortex_query_explain` carries lane timings + drops only
    because the orchestrator does not retain pre-fusion ranked
    lists after `rrf_fuse` runs.

Lesson 2 — **the consolidation-id lookup must accept BOTH keys.** The
producer stamps a stable `consolidation_id` on every envelope; the
envelope's `event_id` changes every re-emit. A naive `event_id="X"`
filter misses re-emitted rows. Every Group B tool uses the same OR
filter `event_id = "X" OR ext.consolidation.consolidation_id = "X"`
in ONE Meili call so the round-trip cost is bounded and re-emitted
consolidations still resolve via the producer id.

Lesson 3 — **per-repo schemas beat global schemas for governance
queries.** The global `cortex_laws` index declares only `severity` +
`applies_to` as filterable, but LawViolation envelopes need
`session_id` / `law_id` / `ts` filters to be useful.
`cortex_law_violations` made `repo` REQUIRED and routes to the
per-repo `cortex-<slug>-governance` index, which inherits the
worker's full filterable set + always pins `kind = "law_violation"`
so law-definition docs that share the family route never leak.

Lesson 4 — **expose the projection mismatch on the wire.** The
`match_strategy` discriminator on Group B / Group C responses
(values: `filter` / `q` / `bm25` / `doc_only` / `envelope_only`)
lets callers detect partial / placeholder shapes without re-issuing
the request. Reserved follow-up values (`hybrid_rrf` /
`with_lane_hits` / `joined`) document the upgrade path inline so a
future schema bump + reindex flips the strategy label without a
caller migration.

How verified: registry size lands at 29 (count assertions in
`tools.rs::tests`, `server.rs::tests`, `transport_stdio.rs::tests`
all green). 100+ unit tests across 16 handlers + 78 wiremock IT
files (one per tool, every IT covers happy / invalid-input /
api_unreachable / descriptor pin). `cargo check --workspace` clean,
`cargo clippy -p cortex-mcp-server -p cortex-api -- -D warnings`
clean, `cargo fmt --check` clean. Specs 22 / 18 / 27 updated with
the per-tool wire shape, the error taxonomy, and the daemon-side
cross-references.

Action items NOT in scope of this phase (each documented as the
reserved `match_strategy` value or a `bad_input` rejection):
  - Writer-side projection of `source_event_ids` /
    `source_session_ids` / `cost_cents` / `prompt_tokens` /
    `completion_tokens` on the consolidation Meili doc (unlocks
    full `cortex_consolidation_lineage` + `cortex_consolidation_costs`).
  - `kinds` filter on `Scope` so `cortex_query` can be scoped to
    `kind=consolidation` (unlocks hybrid RRF on
    `cortex_consolidations_search`).
  - Orchestrator change to retain pre-fusion ranked lists per lane
    (unlocks `cortex_query_explain` `with_lane_hits` shape).
  - `repo` column on `pre_thinking_feedback` SQLite table (unlocks
    `cortex_feedback_signals` `repo` filter).
  - `supersedes` / `superseded_by` filterable on the global
    `cortex_decisions` schema (unlocks
    `cortex_decision_search`'s rejected filters).
