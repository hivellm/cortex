## 1. Feedback endpoint + storage
- [ ] 1.1 New SQLite table `pre_thinking_feedback { query_id, helpful, files_cited, rating, free_text, recorded_at }`. Migration in `cortex-storage::metadata`.
- [ ] 1.2 New `POST /v1/pre-thinking/feedback` route in `cortex-api/src/feedback.rs`. Validates the `query_id` exists in the audit table.
- [ ] 1.3 Idempotent: re-posting the same `query_id` overwrites prior feedback for that turn.
- [ ] 1.4 Unit tests: 4 cases (happy path, unknown query_id rejected, idempotent overwrite, malformed body 400).

## 2. Per-intent budget config
- [ ] 2.1 Extend `cortex_config::PreThinkingConfig` with `budget_per_intent: HashMap<Intent, ByteSize>`.
- [ ] 2.2 Defaults: `pre_change_context = 32KB`, `explain = 24KB`, `decision_lookup = 24KB`, `similar_problems = 32KB`, `law_check = 12KB`, `coverage = 16KB`.
- [ ] 2.3 `BudgetClipper::clip(intent, sections)` reads the per-intent cap.
- [ ] 2.4 Round-trip test: each intent honours its cap in TOML + env-var form.

## 3. Implicit feedback signal
- [ ] 3.1 New `cortex-pre-thinking::implicit_feedback::detect_citation(turn_first_tokens, bundle_files) -> JaccardScore`.
- [ ] 3.2 Feedback recorder calls it on every Turn envelope and stores the score in `pre_thinking_feedback.implicit_score`.
- [ ] 3.3 IT pinning the score on a known fixture turn.

## 4. Metrics + dashboard
- [ ] 4.1 Histogram `cortex_pre_thinking_bundle_bytes_per_intent` segmented by `intent` label.
- [ ] 4.2 Counter `cortex_pre_thinking_helpful_total{intent, helpful}` driven by feedback POSTs.
- [ ] 4.3 New dashboard view `Pre-Thinking Quality` showing: per-intent bundle-size p50/p95/p99, helpful_rate, files_cited_rate, implicit_score distribution.
- [ ] 4.4 GUI snapshot test for the new view.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/12-pre-thinking-injection.md` § Feedback + § Per-intent budget + `CHANGELOG.md`.
- [ ] 5.2 Tests: §1.4 + §2.4 + §3.3 + §4.4.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C gui test` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
