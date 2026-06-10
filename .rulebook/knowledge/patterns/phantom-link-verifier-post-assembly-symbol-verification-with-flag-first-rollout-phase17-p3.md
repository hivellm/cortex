# Phantom-link verifier: post-assembly symbol verification with flag-first rollout (phase17 P3)

**Category**: architecture
**Tags**: phase17, phantom-link, verifier, tree-sitter, retrieval, flag-then-filter

## Description

Post-snippet-assembly pass in the orchestrator verifies every cited (path, symbol) pair against the working tree before results reach the model. verify_symbol dispatches by extension: .rs → tree-sitter parse + recursive walk of named items (fn/struct/enum/trait/impl/mod/type/const/static); .md → string scan of ATX heading slugs (GitHub anchor format) + code-fence identifier lines; anything else → Unsupported (never flagged/dropped). LRU file-content cache (1000 entries, Mutex<LruCache<PathBuf, Arc<String>>> behind OnceLock) keeps hot paths off disk. Load-bearing config (VerifyConfig): symbols_enabled=true, action="flag" (attach verified/verdict metadata, keep snippet) — switch to "filter" (drop unverified) only after ~2 weeks of measuring phantom-link rate via the `phantom_link_dropped` audit event. Env knobs: CORTEX_VERIFY_{SYMBOLS_ENABLED,ACTION}. Verdicts: Verified/NotFound/FileMissing/Unsupported, serialized snake_case on the Snippet wire shape. Eval gate (phantom rate ≤1%) pending live stack — phase17 §3.10 blocked.

## Example

apply_phantom_link_verification(&mut snippets, workspace_root, &cfg.action, &query_id) — crates/cortex-api/src/search/orchestrator.rs; resolver in crates/cortex-workers/src/verify/symbols.rs

## When to Use

Whenever retrieved content cites repo artifacts that can drift (renamed symbols, deleted files): verify at serve time against the live tree, roll out observe-only first ("flag"), enforce ("filter") only after measuring false-positive rate.

## When NOT to Use

Don't gate on verification for content types without a resolver — mark Unsupported and pass through; dropping unverifiable-by-construction snippets silently starves retrieval.
