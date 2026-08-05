# Proposal: phase30b_consolidations-read-path

## Why

phase30_continuity §1.1/§1.2 proved live that the cross-session
continuity loop is broken at the retrieval layer: `/v1/query` NEVER
populates `results.consolidations`, so the pre-thinking renderer's
"Consolidated context" section — the mechanism that carries a prior
session's distillate into a fresh session — cannot render in
production (it only ever renders from hand-built unit fixtures).

Three independent breaks, verified live 2026-08-05:

1. **Keyword lane queries an extinct index.** The
   `pre_change_context` / `similar_problems` plans fan out to the
   global `INDEX_CONSOLIDATIONS` (`cortex_consolidations`,
   `crates/cortex-storage/src/names.rs:141`) — Meili answers
   `index_not_found`; consolidations are written to per-repo
   `cortex-<repo>-consolidations` (62 docs in
   `cortex-cortex-consolidations` live). The strategies comment
   ("no per-repo consolidation index exists yet", phase11j) predates
   the routing change that moved writes per-repo.
2. **Vector lane queries a missing collection.**
   `COLLECTION_CONSOLIDATION_FP32` (`cortex.consolidation.fp32`,
   names.rs:99) does not exist in the live Vectorizer (404; 181
   collections checked) — consolidations are apparently never
   embedded into a dedicated collection.
3. **No assembly.** Even when a consolidation hit comes back through
   any lane, the orchestrator maps every hit into `results.snippets`
   (`snippet_from_hit`, orchestrator.rs) — NOTHING in cortex-api
   constructs `ConsolidationRef` / fills `results.consolidations`
   (`rg 'ConsolidationRef \{' crates/cortex-api/src` → only the type
   definition; constructors exist only in cortex-pre-thinking test
   fixtures).

Red proof: `cargo test -p cortex-pre-thinking --test
cross_session_continuity_it -- --ignored` (live-gated, `#[ignore]`)
drives the production pipeline against the live stack and fails at
the continuity assert with a real (consolidation-less) bundle.

## What Changes

1. Strategies: fan the consolidations keyword lane out to the
   per-repo `cortex-<repo>-consolidations` uid (mirror the
   `repo_scoped` helper used for code/docs), keeping the global uid
   only if a global index is re-introduced deliberately.
2. Orchestrator: partition consolidation-kind hits out of the
   snippet stream and assemble `ConsolidationRef` entries
   (consolidation_id, grain, ts, title, outcome) into
   `results.consolidations` — the fields exist on the Meili docs.
3. Vector lane: decide explicitly — either wire consolidation
   embedding into a real `cortex.consolidation.fp32` collection
   (embedder routing) or REMOVE the phantom collection from the
   plans; no half-wired lane.
4. Un-ignore `cross_session_continuity_it` — it is the acceptance
   gate for this task.

## Impact

- Affected specs: docs/specs/12-pre-thinking-injection.md (Consolidated
  context section becomes real), docs/specs/11-query-api.md if present
  (results.consolidations contract)
- Affected code: crates/cortex-api/src/search/strategies.rs,
  crates/cortex-api/src/search/orchestrator.rs,
  crates/cortex-storage/src/names.rs (if uid scheme changes),
  crates/cortex-pre-thinking/tests/cross_session_continuity_it.rs
- Breaking change: NO (additive population of an always-empty field)
- User benefit: "the agent forgot what we just did" stops being
  structural — prior-session consolidations actually reach fresh
  sessions' bundles.
