## §1. Document the current feedback + ranking state (no code changes)
- [ ] §1.1 Confirm and record the exact `pre_thinking_feedback` schema (`query_id, intent, helpful, files_cited, rating, free_text, implicit_score, recorded_at` — `crates/cortex-storage/src/metadata.rs::apply_phase14f_schema`) and note explicitly that it carries no lane/repo column.
- [ ] §1.2 Confirm and record that `crates/cortex-api/src/search/fusion.rs` and `strategies.rs` contain zero references to feedback signals today — ranking is entirely unaffected by recorded `helpful`/`unhelpful` values.
- [ ] §1.3 Catalogue `FusionConfig`'s existing per-hit multiplier precedents (`cross_repo_boost`, `same_session_boost`/`cohort_session_boost`, `outcome_multiplier`) and the `hit.extras["source"]` lane-identity field as the pattern and join key this task extends.

## §2. Design lane attribution + decayed weight adjustment
- [ ] §2.1 Design how a bundle-level `helpful`/`unhelpful` signal gets credited to the lane(s) responsible — e.g. persist per-`query_id` lane composition (`doc_id -> source lane`) at bundle-assembly time, then at feedback-record time diff `files_cited` against each lane's contributed `doc_id`s to derive per-lane credit/blame.
- [ ] §2.2 Design the exponential-decay update rule for a `(intent, lane) -> weight` table (decayed running average, bounded range e.g. `[0.5, 1.5]`, so a small number of signals never drives a lane's contribution to zero).
- [ ] §2.3 Decide persistence: new SQLite table in `cortex-storage::metadata`, following the `pre_thinking_feedback` table's own precedent.

## §3. Implement gated behind a feature flag
- [ ] §3.1 Add `FeedbackLoopConfig` (enabled flag default `false`, env-var override, mirroring `RerankerConfig`'s existing shape) in `crates/cortex-config`.
- [ ] §3.2 Extend `FusionConfig` with the per-(intent, lane) weight lookup and apply it as a multiplier inside `rrf_fuse`, keyed on `hit.extras["source"]`, using the same accumulate-then-multiply structure as `cross_repo_boost` / `outcome_multiplier`.
- [ ] §3.3 Wire the orchestrator to load the current weight table and merge it into the per-request `FusionConfig` at the existing pre-`rrf_fuse` construction site.
- [ ] §3.4 Wire `cortex_feedback_record` (or a small background recompute pass over `pre_thinking_feedback`) to keep the weight table updated from live signals per §2.2's decay rule.

## §4. Make the effect measurable
- [ ] §4.1 Reference `phase28_retrieval-eval-gate-live` (its own scope stays in that task) — plan the on-vs-off comparison as a follow-on once that gate is live.
- [ ] §4.2 Surface the active per-(intent, lane) weight multipliers through `cortex_query_explain`'s existing `fusion_math` projection so the effect is inspectable immediately, ahead of the full eval gate.

## §5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §5.1 Update or create documentation covering the implementation (spec 11 fusion section, `cortex_feedback_record`/`cortex_feedback_signals` MCP tool-surface docs, the new `FeedbackLoopConfig` env vars)
- [ ] §5.2 Write tests covering the new behavior (lane-attribution unit tests, decay-and-bounds unit tests, `rrf_fuse` multiplier tests, orchestrator wiring test, `query_explain` projection test)
- [ ] §5.3 Run tests and confirm they pass
