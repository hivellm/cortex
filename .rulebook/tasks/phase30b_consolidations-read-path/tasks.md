## 1. Implementation
- [ ] 1.1 Keyword lane: point the consolidations fan-out in `pre_change_context` / `similar_problems` plans at the per-repo `cortex-<repo>-consolidations` uid (the write-side reality; the global `cortex_consolidations` is extinct live). Honour `scope.repo`; update the two strategy unit tests that assert the uid.
- [ ] 1.2 Assembly: in the orchestrator, partition consolidation-kind lane hits out of the snippet stream and build `ConsolidationRef` (consolidation_id, grain, ts, title, outcome — all present on the Meili docs) into `results.consolidations`, capped per the formatter's `consolidations_cap`.
- [ ] 1.3 Vector lane decision: either wire consolidation embedding into a real `cortex.consolidation.fp32` collection (embedder routing + backfill of the 62 existing docs) or remove the phantom collection from the plans — no half-wired lane. Record the decision rationale here.
- [ ] 1.4 Acceptance: remove `#[ignore]` from `cross_session_continuity_it::prior_session_consolidation_surfaces_in_fresh_session_bundle` and confirm it passes against the live stack (a prior-session consolidation surfaces in a fresh session's bundle).

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
