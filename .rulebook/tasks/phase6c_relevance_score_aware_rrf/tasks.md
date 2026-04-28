## 1. Score normalisation helper
- [ ] 1.1 In `crates/cortex-api/src/lanes.rs`, add `impl LaneHit { pub fn normalized_score(&self) -> f32 { self.score.clamp(0.0, 1.0) } }` with rustdoc explaining the `[0,1]` contract
- [ ] 1.2 Confirm Vectorizer + Meili lanes already emit scores in `[0,1]` (cosine + `_rankingScore`); no change required there
- [ ] 1.3 Document in `lanes.rs` the graph-lane caveat: today `score = 0.0`, post-`phase4c` SHALL stamp a path-length-derived score (out of scope here)

## 2. Score-aware fusion
- [ ] 2.1 In `crates/cortex-api/src/fusion.rs`, change `rrf_fuse` to accept a `FusionConfig { alpha: f32, k: u32 }` parameter (or a static accessor reading the global `FusionConfig`) instead of the hard-coded `K = 60`
- [ ] 2.2 Implement the blend: `fused = alpha * (1.0 / (k as f32 + rank as f32)) + (1.0 - alpha) * hit.normalized_score()`
- [ ] 2.3 Sum across lanes per hit id; stable sort descending by fused score, then by lane priority for tiebreak (today's behaviour)
- [ ] 2.4 Boundary checks: clamp `alpha` to `[0.0, 1.0]` at construction; clamp `k` to `>= 1`

## 3. Configuration
- [ ] 3.1 In `crates/cortex-api/src/main.rs`, read `CORTEX_RRF_ALPHA` (parse as `f32`, default `0.7`, log + fall back to default on out-of-range)
- [ ] 3.2 Read `CORTEX_RRF_K` (parse as `u32`, default `60`, log + fall back to default on `<= 0`)
- [ ] 3.3 Build a `FusionConfig` and inject it through `Orchestrator::new` so handlers + tests share the same source of truth

## 4. Audit envelope
- [ ] 4.1 Extend `AuditEnvelope` with `fusion_alpha: f32` and `fusion_k: u32`
- [ ] 4.2 Stamp them from the resolved `FusionConfig` on every audit emit
- [ ] 4.3 Update the audit fixture in `crates/cortex-api/tests/http.rs` to assert both fields are present

## 5. Regression tests
- [ ] 5.1 In `fusion.rs::tests`, add `weak_graph_hit_does_not_outrank_dense_vector_top3` per §What Changes
- [ ] 5.2 Add `all_equal_native_scores_reduce_to_positional_rrf` — assert fused order matches pure-positional baseline within `1e-6`
- [ ] 5.3 Add `alpha_one_reproduces_positional_only` (regression escape hatch) and `alpha_zero_sorts_by_native_score`
- [ ] 5.4 Update existing fusion tests that depended on the old hard-coded constant: parametrise with `FusionConfig::default()` so they continue to pass

## 6. Spec docs
- [ ] 6.1 In `docs/specs/11-query-api.md`, add a "Fusion algorithm" subsection documenting the blend formula, the `α` / `K` env knobs, the audit fields, and the per-lane normalised-score convention
- [ ] 6.2 Cross-link from `docs/analysis/relevance/01-findings.md` §F-005 (mark closed-by phase6c on merge)

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` per §6
- [ ] 7.2 Write tests covering the new behavior — the four regression tests in §5 plus the audit fixture extension in §4
- [ ] 7.3 Run tests and confirm they pass — `cargo clippy -p cortex-api --all-targets -- -D warnings` and `cargo test -p cortex-api` both green
