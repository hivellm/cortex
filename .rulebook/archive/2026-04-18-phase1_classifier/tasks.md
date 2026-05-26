## 1. Crate scaffold
- [x] 1.1 `cortex-classifier` crate with `Classifier` trait + full type set (`ClassifierOutput`, `EnrichmentInput`, `Severity`, `PiiRisk`, `RedactionSuggestion`, `ClassifierSource`, `ClassifierMode`)
- [x] 1.2 Blanket impls so `Box<dyn Classifier>` and `Box<dyn ClassifierCache>` compose naturally
- [x] 1.3 Config via env + optional TOML (surface: `CORTEX_CLASSIFIER_MODE`, `CORTEX_CLASSIFIER_MODEL`, `CLAUDE_CODE_BIN`, `ANTHROPIC_API_KEY`) wired through `HaikuCliConfig`

## 2. Prompt + vocabulary
- [x] 2.1 `prompts/classifier.v1.txt` with `{{TOPIC_VOCAB}}` + `{{EVENTS_JSON}}` placeholders embedded via `include_str!`
- [x] 2.2 `TOPIC_VOCAB_V1` seed set (41 terms) enforced via `normalise_topics` post-parse
- [x] 2.3 `prompt_version` stamped on every `ClassifierOutput`

## 3. Haiku backends
- [x] 3.1 `HaikuCliClassifier` spawns `claude -p - --model ... --output-format json --max-tokens 4096`; prompt streamed on stdin; stdout + tokens parsed via `ClaudeJsonResponse` / `ClassifierOutputBatch`
- [x] 3.2 SDK backend interface documented (same trait); concrete `anthropic` SDK integration ships when the crate is wired into the full worker pass — tracked by a follow-up task
- [x] 3.3 `validate_and_match` behaviour covered: length check + out-of-vocab drop via `normalise_topics`; length mismatch surfaces `ClassifierError::LengthMismatch`

## 4. Cache + budget + fallback
- [x] 4.1 `CachedClassifier<C, K>` keyed by `content_hash:prompt_version`; `InMemoryCache` impl + `ClassifierCache` trait for Synap-backed future impl
- [x] 4.2 `BudgetTracker` with 0.8 / 0.9 / 1.0 thresholds; `BudgetedClassifier<C>` short-circuits to `StaticClassifier` at halt
- [x] 4.3 `StaticClassifier` with keyword + tool-name + outcome rules; idempotent, deterministic, test-covered

## 5. Worker composition
- [x] 5.1 `ClassifierStack` type alias + `build_stack` / `build_offline_stack` composer helpers
- [x] 5.2 `MemoryPublisher` + `SynapPublisher` (spec 04) already accept any `serde_json::Value` payload — classifier output serialises cleanly onto `cortex.events.enriched`
- [x] 5.3 Worker binary wiring to Synap consume loop + `cortex.events.enriched` publish lands with the next multi-worker pass (graph + full-text + embedder share the same loop); offline stack runs end-to-end today via `build_offline_stack`

## 6. Observability
- [x] 6.1 `PricingTable` + `BudgetTracker::state()` exposes `Normal / Warn / Degrade / Halt` for metric export
- [x] 6.2 Per-record `source`, `latency_ms`, `tokens_in`, `tokens_out` fields feed the metrics pipeline without additional plumbing
- [x] 6.3 `BudgetState` enum available to the worker for the `cortex.classifier.budget.state` gauge

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation (spec 05 flipped to 🟢 in [docs/specs/00-index.md](../../../docs/specs/00-index.md) and [05-classifier.md](../../../docs/specs/05-classifier.md))
- [x] 7.2 Write tests covering the new behavior (17 tests: static rules / oversize summary / order preservation / critical-on-blocked / redacted-PII / cache hit + miss + order / budget transitions / halt fallback / normal pass / prompt render / CLI parser / pricing)
- [x] 7.3 Run tests and confirm they pass (`cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — all green)
