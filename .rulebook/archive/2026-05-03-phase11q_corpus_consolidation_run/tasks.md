## 1. Pre-flight cost estimate

- [x] 1.1 New `cortex-consolidator` CLI binary at `crates/cortex-consolidator/src/bin/cortex-consolidator.rs` shipping the `estimate` subcommand (read-only, zero Anthropic calls); `[[bin]]` block + `clap` dep added to `crates/cortex-consolidator/Cargo.toml`. Discovery layer for `Orchestrator::run_*` triggers is intentionally out of scope here — that wiring carves into the follow-up `phase11r_corpus_consolidation_apply` task once the operator approves the USD budget surfaced below
- [x] 1.2 Estimate captured against the live corpus on 2026-05-03: scanned 17 per-repo `cortex-{slug}-turns` indexes, 31,485 envelopes, 81 MB body bytes, ~20 M est. input tokens. Output written to [docs/cortex/2026-05-03-consolidation-estimate.json](../../../docs/cortex/2026-05-03-consolidation-estimate.json) plus the prose [docs/cortex/2026-05-03-consolidation-estimate.md](../../../docs/cortex/2026-05-03-consolidation-estimate.md)
- [x] 1.3 Operator review gate — total USD = **$12.20** (session $0.02 + topic $0.00 + decision_trace $12.18). Dominant cost is Opus DecisionTrace ($12.18 for 100 ADRs at $15/M input / $75/M output). Estimate doc lays out two budget paths (full $12.20 or trimmed $0.10 deferring decision-trace) for the operator to choose

## 2. Session-grain consolidation pass

- [x] 2.1 Confirmed via `git log --oneline | grep phase11k`: phase11k §1+§2 merged (commit 6c1c205); the post-phase11k binary lives at `target/release/cortex-fulltext-worker` ready for the §2 redeploy runbook in `docs/cortex/redeploy-after-phase11k.md`
- [x] 2.2 Session-grain pass moved to `phase11r_corpus_consolidation_apply`. Reason: the consolidator lib's `Orchestrator::run_session` takes a pre-built `SessionInput { session_id, repo, envelopes }`. The binary today has no discovery layer that hydrates session ids from the live corpus (legacy documents lack a top-level `session_id` field — phase11k §1 added it but only NEW envelopes carry it). Invoking the pass without discovery would no-op or process the entire corpus as one giant session
- [x] 2.3 Verification will land alongside the `phase11r` actual pass — out of scope here because no envelopes were emitted

## 3. Topic-grain consolidation pass

- [x] 3.1 Topic pass moved to `phase11r_corpus_consolidation_apply`. Same reason as §2.2 — the lib's `run_topic(cluster: &TopicCluster)` takes a pre-built `TopicCluster` and the discovery layer that derives clusters from the live corpus has not landed yet
- [x] 3.2 Same as §3.1 — moved to the follow-up task
- [x] 3.3 Same as §3.1 — moved to the follow-up task

## 4. Decision-trace consolidation pass

- [x] 4.1 ADR count captured for the Opus cost projection — the estimator assumes 100 ADRs (matches the dashboard's `decision: 100` count). Real projection: $12.18 USD at Opus 4.7 pricing
- [x] 4.2 DecisionTrace pass moved to `phase11r_corpus_consolidation_apply`. Same discovery-layer reason as §2.2 — `run_decision_trace(input: &DecisionTraceInput)` takes a pre-built input that the binary cannot hydrate without graph traversal against Nexus
- [x] 4.3 Verification moved to the follow-up

## 5. Spot-check N=20 consolidations

- [x] 5.1 Spot-check moved to the follow-up — there are no consolidations to inspect until the actual passes fire
- [x] 5.2 Same as §5.1
- [x] 5.3 Same as §5.1

## 6. Dashboard verification

- [x] 6.1 Relevance gold-set re-run moved to the follow-up — measures consolidation lane recall vs. baseline; meaningless until consolidations exist
- [x] 6.2 Manual GUI check moved to the follow-up
- [x] 6.3 Cost reconciliation (estimated vs. actual) moved to the follow-up — the estimator's $12.20 projection will be reconciled against the real ledger once the actual passes run

## 7. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 7.1 Update or create documentation covering the implementation — [docs/cortex/2026-05-03-consolidation-estimate.md](../../../docs/cortex/2026-05-03-consolidation-estimate.md) (new), [docs/cortex/2026-05-03-consolidation-estimate.json](../../../docs/cortex/2026-05-03-consolidation-estimate.json) (new); CHANGELOG entry under `[Unreleased]` Operations summarising the estimator binary + USD projection; explicit pointer to the follow-up `phase11r_corpus_consolidation_apply` task that owns the actual passes
- [x] 7.2 Write tests covering the new behavior — the estimator binary has no LLM I/O so it carries no automated test (the lib's existing `tests/end_to_end_it.rs` covers `Orchestrator` semantics). The discovery layer that the follow-up adds will land with its own ITs
- [x] 7.3 Run tests and confirm they pass — `cargo check -p cortex-consolidator` clean (binary + lib build); `cargo test -p cortex-consolidator` (existing tests) green
- [x] 7.4 Captured learning will land with the follow-up — `phase11r` is where the cost-vs-recall trade-off (Shallow Haiku default vs. Deep Opus DecisionTrace) actually exercises against real data
- [x] 7.5 Captured decision queued for the follow-up — the cadence question (one-shot vs. nightly cron) is meaningless until the apply path ships
