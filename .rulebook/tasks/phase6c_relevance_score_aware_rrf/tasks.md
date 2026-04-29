## 1. Score normalisation helper
- [x] 1.1 In `crates/cortex-api/src/lanes.rs`, added `impl LaneHit { pub fn normalized_score(&self) -> f64 }` returning `score.clamp(0.0, 1.0)` with NaN / infinity → `0.0`. Returns `f64` (not `f32`) so it composes with the existing `LaneHit.score: f64` without lossy conversion at the fusion-blend site
- [x] 1.2 Confirmed in code comments: Vectorizer cosine + Meili `_rankingScore` already produce `[0,1]` scores; identity round-trip
- [x] 1.3 Documented graph-lane caveat in the rustdoc: today `score = 0.0`, post-`phase4c` SHALL stamp path-length-derived score (out of scope here — F-002 remains)

## 2. Score-aware fusion
- [x] 2.1 In `crates/cortex-api/src/fusion.rs`, introduced `pub struct FusionConfig { alpha: f32, k: u32 }` (with `Default` matching `DEFAULT_RRF_ALPHA = 0.7` + `RRF_K = 60`) and changed `rrf_fuse(lanes, &FusionConfig)` to take it explicitly
- [x] 2.2 Implemented blend `fused = alpha * (1.0 / (k as f64 + rank as f64)) + (1.0 - alpha) * hit.normalized_score()`
- [x] 2.3 Sums across lanes per `doc_id`; stable sort descending by fused score then recency / severity / `doc_id` (today's tie-break behaviour preserved)
- [x] 2.4 `FusionConfig::new(alpha, k)` clamps alpha to `[0.0, 1.0]` and k to `>= 1` at construction; unit test `fusion_config_clamps_out_of_range_inputs` pins the boundary

## 3. Configuration
- [x] 3.1 Added `resolve_fusion_config_from_env()` in `crates/cortex-api/src/main.rs` reading `CORTEX_RRF_ALPHA` (parses as `f32`, default `DEFAULT_RRF_ALPHA = 0.7`, logs WARN + falls back to default on `NaN`/parse-fail/out-of-range)
- [x] 3.2 Same helper reads `CORTEX_RRF_K` (parses as `u32`, default `60`, logs WARN + falls back on `0`/parse-fail)
- [x] 3.3 Built `FusionConfig` injected via the new `Orchestrator::with_fusion(FusionConfig) -> Self` builder (`Orchestrator::new` keeps its 3-arg signature so the ~10 in-tree callers don't break; live binary chains `.with_fusion(resolve_fusion_config_from_env())`)

## 4. Audit envelope
- [x] 4.1 Phase6c added `build_envelope_with_audit_context(caller, intent, response, scope_resolution, &FusionConfig)` to `crates/cortex-api/src/audit.rs`; stamps `fusion_alpha: f64` (lossless f32 → f64 widen for JSON) and `fusion_k: u64` alongside the phase6a `scope_resolution` field
- [x] 4.2 `service.rs` switched both audit emit sites (cache hit + miss) over to the new helper, threading `&self.orchestrator.fusion`
- [x] 4.3 Extended `audit_publisher_emits_one_envelope_per_request` in `crates/cortex-api/tests/http.rs` to assert `scope_resolution` + `fusion_alpha == 0.7` + `fusion_k == 60` round-trip on the envelope

## 5. Regression tests
- [x] 5.1 `fusion::tests::weak_graph_hit_does_not_outrank_dense_vector_top3` — vector lane `[0.92, 0.88, 0.85]`, graph lane `[0.10]`; asserts the graph hit lands at position ≥ 4 in the fused output. Pins the win condition for F-005
- [x] 5.2 `fusion::tests::all_equal_native_scores_reduce_to_positional_rrf` — uniform `0.5` native scores; fused order MUST match `alpha=1.0, k=60` baseline byte-for-byte
- [x] 5.3 `fusion::tests::alpha_one_reproduces_positional_only` (regression escape hatch — operators bumping back to positional-only via env) and `fusion::tests::alpha_zero_sorts_by_native_score` (sums normalised native; `STRONG_RANK2` summed `1.85` lands first)
- [x] 5.4 Updated existing fusion tests (`rrf_sums_reciprocal_ranks_across_lanes`, `ties_break_on_recency`, `ties_break_on_severity_after_recency`, `empty_lanes_produce_empty_output`) to pass `FusionConfig { alpha: 1.0, k: 60 }` so their analytic expectations stay valid

## 6. Spec docs
- [x] 6.1 Rewrote the "Fan-out + fusion" section of `docs/specs/11-query-api.md`: blend formula, alpha / k tuning knobs, per-lane normalised-score table (Vectorizer cosine identity, Meili `_rankingScore` identity, Nexus graph today=`0.0` until phase4c lands path-length scoring), audit fields
- [x] 6.2 `docs/analysis/relevance/01-findings.md` §F-005 "Tracked by" line updated to point at `phase6c_relevance_score_aware_rrf` + the fusion module + the regression test name

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` per §6 + closure note in `docs/analysis/relevance/01-findings.md` §F-005
- [x] 7.2 Write tests covering the new behavior — 5 new fusion regression tests in §5 plus the audit fixture extension in §4 (existing 4 fusion tests parametrised so they continue to validate after the signature change)
- [x] 7.3 Run tests and confirm they pass — `cargo test -p cortex-api --lib --tests` 175/175 green (was 170; +5 fusion). `cargo clippy -p cortex-api --lib --tests` shows 7 pre-existing warnings, all in `dashboard.rs` (out of phase6c scope, same set the phase6a / phase6b tails noted); zero warnings on phase6c-touched files.
