## 1. Reorder + split intent rules
- [ ] 1.1 Audit `DEFAULT_RULES`: identify rules whose substring is a prefix of other rules. Reorder so longer / more-specific patterns match first.
- [ ] 1.2 Add 12 new compound rules covering observed mismatches (`why did we`, `decided to pick`, `chose to`, etc.).
- [ ] 1.3 Per-rule unit test: 5 fixture prompts per intent, verifying correct routing.
- [ ] 1.4 Regression test asserting `"explain why did we pick hnsw"` routes to `decision_lookup`, not `explain`.

## 2. Mismatch metric
- [ ] 2.1 New counter `cortex_pre_thinking_intent_mismatch_total{from, to}` registered in `metrics.rs`.
- [ ] 2.2 Increment on every feedback row whose `helpful = false` AND the bundle's intent differs from the intent the model corrected to in the same turn.
- [ ] 2.3 New `cortex-ops intent-stats [--since 7d]` subcommand prints per-intent mismatch rate.

## 3. Cascade rewriter
- [ ] 3.1 New `cortex-pre-thinking::rewriter::cascade(query, intent) -> RewrittenQuery`. Tries Sonnet with 800ms timeout + response cache; falls through to deterministic on any failure.
- [ ] 3.2 Sonnet cache: SHA256(query + intent) → rewritten_query, TTL 24h, capped at 10k entries.
- [ ] 3.3 Telemetry: `cortex_pre_thinking_rewriter_path_total{path}` with paths `sonnet_hit`, `sonnet_miss`, `sonnet_cache_hit`, `sonnet_timeout`, `deterministic_fallback`.
- [ ] 3.4 Default `CORTEX_PRE_THINKING_REWRITER = "cascade"`. Docs explain the trade-off.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/12-pre-thinking-injection.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: §1.3 × 6 intents + §1.4 + §3 cascade unit tests (sonnet OK, sonnet timeout falls through, cache hit short-circuits).
- [ ] 4.3 `cargo check --workspace && cargo clippy -p cortex-pre-thinking -- -D warnings && cargo test -p cortex-pre-thinking` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
