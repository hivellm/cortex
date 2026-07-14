## 1. Reconcile and audit the golden fixture trees
- [x] 1.1 Canonical = `crates/cortex-eval/tests/golden/` (real recent ULIDs, backfilled mcp rows, only tree with access_control.csv; root tree was stale phase14c-era content). `.github/workflows/eval.yml --golden`, the binary's `--golden` default, and every doc reference now point at the crate tree.
- [x] 1.2 Root `tests/golden/` retired (`git rm`); its still-relevant curation targets (retrieval 100 / consolidation 50 / classification 200, quarterly + per-incident cadence, RFC-4180 tips) merged into the canonical README before deletion.
- [x] 1.3 Backfilled 2 rows with live-harvested ids after fixing their root causes: **m-003 `cortex_tool_calls`** — the handler filtered on flat `tool_name`/`outcome` but live docs carry the NESTED `ext.tool_call.*` shape and only the flat names were filterable → settings v9 adds `ext.tool_call.tool_name`/`.outcome` to filterableAttributes (applied live to `cortex-cortex-code`, verified: 1000 Bash hits), handler repointed to nested paths (row pins the low-frequency `WebSearch` so ts-sorted top-5 stays stable); **m-008 `cortex_consolidations_by_entity`** — the eval driver omitted the `repo` hint the API requires (no global consolidations index) → driver fixed, ADR-016 ids harvested live. Kept dropped with findings recorded: `files_touched` (works but full-archive scan ≈30s/call AND `repo:"cortex"` matches zero envelopes in any window — `context.repo` value mismatch, real defect), `topic_search` (0 topic_card docs in the live corpus), `law_violations` (0 law_violation docs — only 274 law definitions), plus the 7 tools without top-K-id semantics (already excluded by the driver by design).
- [x] 1.4 Confirmed intentionally synthetic: `access_control.csv` is a Bell-LaPadula truth-table over (clearance × grants × fact-label) driving the zero-false-grant gate — a predicate matrix has no live counterpart to harvest. Already lives only in the canonical tree; README row added documenting this.

## 2. Grow the golden set to a statistically meaningful size
- [x] 2.1 retrieval.csv 18 → **27** rows (every new row's expected value live-verified in the top-10 of an actual `/v1/query` under its own intent; ~4 candidates discarded because the correct answer did not surface); consolidation.csv 6 → **10** rows (real consolidation ULIDs harvested from `/v1/consolidations/recent`, entities/facts verified as literal substrings of the live summaries). Long-term targets (100/50) remain future curation cadence.
- [x] 2.2 New `intent` column (5-column schema; `#[serde(default)]` keeps 4-column fixtures loading, blank → free_search) + driver sends the row intent. All 27 rows tagged; coverage: pre_change_context 6, decision_lookup 5, similar_problems 2, law_check 2, free_search 12. **Two intent-semantics discoveries fixed en route:** (a) `law_check` strips `results.snippets` BY SPEC (only `laws_active` + violations surface) — law_check rows are keyed by law id and the driver reads `laws_active[].id`; (b) `similar_problems` fans out to the turns corpora whose hits carry NO `path` — added the additive `doc_id` field to the wire `Snippet` (lane-level document id, skip-serialising) and the driver falls back `path → doc_id`, rows keyed by `meili|cortex-<repo>-turns|<event_ulid>`. Three rows the sub-agent had mis-tagged similar_problems (docs-path expectations that can never surface there) retagged to free_search.
- [x] 2.3 classification.csv 10 → **26** rows: ≥2 per Kind for ALL 13 variants (the task text undercounted — `Law` was also missing, not just `Analysis`/`TopicCard`); expected_kind uses the loader's snake_case labels.

## 3. Re-run the retrieval eval suite and re-lock the baseline
- [x] 3.1 All FIVE suites run live against the reconciled set (2026-07-14): retrieval 27 rows mrr 0.5864 / recall@5 0.5926; consolidation 10 rows 1.0/1.0; classification 26 rows macro_f1 **1.0**; mcp_search 6 rows 0.667; access_control 40 rows zero false-grants. **The classification suite required implementing the endpoint its driver targets** — no worker ever exposed `POST /v1/classify`, so every row degraded to `Unknown` (macro_f1 0.0). Shipped: `cortex_health::server::serve_standalone_with_metrics_and_router` (extra Router merged onto the admin port), `classifier::http::classify_router()` (serde Kind derivation — the same contract ingestion applies — + StaticClassifier enrichment, 400 on unknown kind), wired in the classifier-worker bin, image rebuilt + redeployed, verified live. Also found live: law_check's `laws_active` at limit 10 does NOT surface the genuinely most-relevant law that limit 5 ranks #1 (r-004/r-019 honestly score 0 — a real ranking gap the gate now tracks), and law-definition docs carry `law_id` slugs truncated at ~59 chars at index time (upstream builder quirk, deterministic).
- [x] 3.2 Real classification entry recorded (26 rows, macro_f1 1.0, real finished_at) — 1970 placeholder gone.
- [x] 3.3 `cdc-baseline-v1.json` re-locked with all five suites' real reports (full per_row detail); historical entries (retrieval_18row, reranked arms) retained; `_note` documents the 2026-07-14 re-lock, the harder-set context, and the known ts-sorted instability of the mcp_search decision_search/events_by_kind rows (their pinned ids age out as new events land — refresh rides the quarterly curation cadence).

## 4. Review and reset the regression-gate floors
- [x] 4.1 `MRR_AT_10_FLOOR` 0.60 → **0.55**, `RECALL_AT_5_FLOOR` 0.50 → **0.55** — both re-derived as baseline − small tolerance (0.5864/0.5926 measured). The old mrr floor was calibrated for the easier 18-row free_search-only set and would have permanently failed the harder multi-intent set; the old recall floor was cleared by a wide margin. All five suites re-run post-change: exit 0.
- [x] 4.2 `MACRO_F1_FLOOR` 0.90 → **0.95** (first real measurement = 1.0; static kind derivation is deterministic, so any drop means a variant stopped parsing — floor set tight).

## 5. Re-enable the nightly schedule
- [x] 5.1 Nightly `schedule:` re-added (03:17 UTC, offset from the cron herd).
- [x] 5.2 `matrix.suite` now carries all five suites — with an honest amendment to the item's premise: on the SCHEDULE event the conditional matrix runs only `classification` + `access_control`, because the other three measure recall against the maintainer's LIVE corpus (turns/consolidations/event ULIDs that a bare CI checkout cannot reproduce) and scheduling them would recreate the red-nightly flood this workflow's own history warns about. Manual dispatch runs all five. The workflow also gained a classifier-worker boot step (the classification suite needs `POST /v1/classify` on :17021 — no prior step ever booted a worker, which is WHY the suite had never produced a real number), and the synap service image moved off `latest` onto `1.0.0`.
- [x] 5.3 Comment block rewritten: documents the reconciled fixtures, the re-derived floors, and the real reason three suites stay dispatch-only (live-corpus dependency, not fixture quality).

## 6. Verify the phase17 P2/P3 gates measure something meaningful
- [ ] 6.1 P2 (reranker, ADR-025 / `docs/specs/37-retrieval-rerank.md`
      §2.7): confirm the already-measured +36% MRR@10 delta still
      holds against the reconciled baseline; add the still-missing
      p95-under-load measurement (≤ +250ms) that was explicitly left
      un-load-tested.
- [ ] 6.2 P3 (phantom-link verifier, ADR-026 /
      `docs/specs/28-phantom-link-verifier.md` §3.10): add a
      phantom-link-rate metric to `cortex-eval` (none exists today —
      the suite currently only measures MRR/recall) and measure it
      against the ≤1% gate.

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
      (`docs/specs/37-retrieval-rerank.md` §2.7 status, `docs/specs/28-phantom-link-verifier.md`
      §3.10 status, new `eval` spec, CHANGELOG)
- [ ] 7.2 Write tests covering the new behavior (phantom-rate metric
      unit tests; reconciled golden CSV structural tests)
- [ ] 7.3 Run tests and confirm they pass
