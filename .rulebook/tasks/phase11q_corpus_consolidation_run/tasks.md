## 1. Pre-flight cost estimate

- [ ] 1.1 Read `crates/cortex-consolidator/src/orchestrator.rs` + `summariser.rs` to confirm the existing dry-run / estimate flag surface; if absent, add `--estimate-only` that walks the cluster discovery path and stops before calling the Anthropic API
- [ ] 1.2 Run the estimate against the live corpus for the three grains: `cortex-consolidator estimate --grain session`, `--grain topic`, `--grain decision-trace`; capture per-cluster token estimates + per-pass USD totals into `docs/cortex/2026-05-03-consolidation-estimate.md`
- [ ] 1.3 Operator review gate — record the total USD, expected output envelope count, and per-grain depth (Shallow / Deep) in the estimate doc; do not proceed past §2 without explicit operator sign-off captured in the doc

## 2. Session-grain consolidation pass

- [ ] 2.1 Confirm the running fulltext-worker + embedder are post-phase11k (so dual-write and v5 settings are active); if not, archive `phase11p_corpus_cleanup_sweep` first
- [ ] 2.2 `cortex-consolidator run-session --all --depth Shallow` against the active corpus; pipe stdout to `target/consolidation/session.log` and the cost ledger to `target/consolidation/session.cost.json`
- [ ] 2.3 Verify each session cluster produced exactly one `Kind::Consolidation` envelope by querying `cortex_consolidations` global Meili index for `grain = "session"` and grouping by `extras.session_id`; flag any session with zero output envelopes for re-run

## 3. Topic-grain consolidation pass

- [ ] 3.1 `cortex-consolidator run-topic --all --depth Shallow` against the active corpus; pipe stdout to `target/consolidation/topic.log` and the cost ledger to `target/consolidation/topic.cost.json`
- [ ] 3.2 Verify topic clusters were derived from the classifier's topic vocabulary (not free-text drift); spot-check 3 topic envelopes to confirm `extras.topics[]` matches the input cluster
- [ ] 3.3 Confirm the new envelopes land in `cortex-{slug}-consolidations` per-repo indexes AND in the global `cortex_consolidations` index (phase11j §3.2 routing)

## 4. Decision-trace consolidation pass

- [ ] 4.1 Pre-check ADR count via `curl /v1/dashboard/decisions?limit=200 | jq '.[] | .id' | wc -l`; expect ~100 entries; record the count in `docs/cortex/2026-05-03-consolidation-run-log.md`
- [ ] 4.2 `cortex-consolidator run-decision --all --depth Deep` against the active corpus; pipe stdout to `target/consolidation/decision.log` and the cost ledger to `target/consolidation/decision.cost.json`; this pass uses Opus 4.7 — monitor cost in real time and abort if the per-trace cost exceeds 2× the estimate
- [ ] 4.3 Verify each ADR with at least one linked turn produced a DecisionTrace envelope by querying `cortex_consolidations` for `grain = "decision_trace"` and joining against the dashboard's decision list; flag any ADR with no consolidation as a graph-link gap (file as a follow-up issue, not blocking this task)

## 5. Spot-check N=20 consolidations

- [ ] 5.1 Sample 5 envelopes from each of session / topic / decision-trace passes plus 5 cross-pass; render each via `cortex-consolidator show <consolidation_id>` (add the subcommand if missing — single SELECT against the per-repo index is enough)
- [ ] 5.2 For each sample, verify: (a) summary captures the cluster's headline, (b) `outcome_distribution` matches the source events' outcome counts, (c) `source_event_count` equals the cluster size, (d) `temporal_span` brackets the cluster's first and last event timestamps
- [ ] 5.3 Capture pass / fail per envelope in `docs/cortex/2026-05-03-consolidation-run-log.md`; on any fail, file a fixture-driven regression test in `crates/cortex-consolidator/tests/`

## 6. Dashboard verification

- [ ] 6.1 Re-run the relevance gold-set (`crates/cortex-cli/tests/relevance_harness_golden.rs`) and capture before / after recall@10 by intent; expect `pre_change_context` and `similar_problems` to gain ≥ 5 percentage points each from the consolidations lane fan-out
- [ ] 6.2 Manual GUI check: open the dashboard, fire a `pre_change_context` query against an active topic, confirm the `## Consolidated context` panel surfaces ≥ 1 consolidation; capture a screenshot in the run-log doc
- [ ] 6.3 Total cost reconciliation: sum the three cost ledgers from §2.2 / §3.1 / §4.2 and record the actual vs. §1.2 estimated USD in `docs/cortex/2026-05-03-consolidation-run-log.md`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 7.1 Update or create documentation covering the implementation — `docs/cortex/2026-05-03-consolidation-estimate.md` (new), `docs/cortex/2026-05-03-consolidation-run-log.md` (new); CHANGELOG entry under `[Unreleased]` Operations summarising envelope count + total cost
- [ ] 7.2 Write tests covering the new behavior — every regression surfaced in §5.3 lands as a fixture-driven test in `crates/cortex-consolidator/tests/`; coverage ≥ 95 % on the consolidator crate after the new fixtures land
- [ ] 7.3 Run tests and confirm they pass — `cargo check -p cortex-consolidator`, `cargo clippy -p cortex-consolidator --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test -p cortex-consolidator`. All green before archive.
- [ ] 7.4 Capture learning: `rulebook_learn_capture` for the corpus-consolidation cost-vs-recall trade-off (Shallow Haiku is the right default; Deep Opus only pays off on DecisionTrace where recall floor matters)
- [ ] 7.5 Capture decision: `rulebook_decision_create` for the cadence question — one-shot now, vs. nightly cron via `cortex-consolidator nightly` once `phase11o_vectorizer_demotion_api` unblocks the pruning half
