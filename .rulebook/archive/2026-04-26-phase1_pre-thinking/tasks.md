## 1. Module scaffold
- [x] 1.1 `cortex-pre-thinking` crate with `PreThinkingInput` + `PreThinkingBudget` + `pipeline::run` entry function (the spec's `cortex-adapters/common/` split lands when spec 17 introduces the per-tool adapters; for v1 the module ships as a workspace crate the Claude Code adapter consumes)
- [x] 1.2 Re-exports under `cortex_pre_thinking` so per-tool adapters import the same surface (Claude Code adapter wires it via `pipeline::run`)
- [x] 1.3 Public error / outcome shape (`PreThinkingOutput` carries `bundle`, `intent`, `query_id`, `steps_applied`, `latency_ms`, `fail_open`) + `Metrics` registry hooks

## 2. Scope derivation
- [x] 2.1 Repo resolution via nearest `.git/` ancestor walk + `cortex.toml` `[cortex] id` override (`scope::repo_from_cwd`)
- [x] 2.2 Files from `recent_files` (age <5 min, `RECENT_FILE_MAX_AGE_SECS`) merged with verbatim prompt mentions via `extract_prompt_files` (regex-bounded to `MAX_PROMPT_FILES = 16`); pure semver tokens like `1.2.3` are filtered out
- [x] 2.3 Topics left empty in v1 per spec 12 Decision 6

## 3. Intent selection
- [x] 3.1 Keyword rule table in `intent_select::DEFAULT_RULES` (decision-lookup phrases, similar-problems debug signals, law-check policy queries, then `pre_change_context` change verbs); fallback is `pre_change_context`
- [x] 3.2 Unit tests cover every rule pair plus the priority-ordering invariant (decision_lookup wins over refactor when both signals appear in the prompt)

## 4. Bundle formatter
- [x] 4.1 Fixed section order in `format_bundle`: laws → decisions → similar turns → snippets → optional graph neighbours
- [x] 4.2 Deterministic pure-Rust string assembly via `std::fmt::Write` + `String::push_str` (no template engine; no model summaries)
- [x] 4.3 Leading comment carries `query_id` for audit correlation; trailing `<!-- end cortex -->` marker
- [x] 4.4 Empty-response → empty string (no "nothing found" placeholder); covered by the `empty_response_returns_empty_bundle` test

## 5. Budget clipper
- [x] 5.1 Section caps in `formatter::section_caps` (laws 10, decisions 5, similar turns 5, snippets 5, graph 0 default; per-snippet 1024 bytes, per-decision 512 bytes)
- [x] 5.2 6-step trim ladder in `clip_to_budget`: `DropGraph` → `SlimSnippets` → `HalveSnippets` → `HalveTurns` → `TruncateDecisions` → `DropSnippets`; ladder exits early as soon as the bundle fits
- [x] 5.3 Laws section is invariant — the budget clipper never trims it; verified by `laws_are_never_dropped` keeping `LAW-007` even at a 600-byte budget

## 6. Failure + audit
- [x] 6.1 Fail-open: `tokio::time::timeout(budget.time_ms, ...)` over the query call → empty string + `fail_open = true` on timeout / `Option::None` from the `QueryFn`; the formatter is wrapped so a panic-shaped error path returns empty
- [x] 6.2 `Metrics` registry covers `calls.total{intent}`, `bundle.bytes`, `sections.count{section}`, `truncation.applied{step}`, `latency_ms`, `empty_bundle`, `timeouts`; structured tracing event emitted on every successful run with `query_id`, `intent`, `bundle_bytes`, `sections`, `steps`
- [x] 6.3 Determinism property: identical input → byte-identical output (verified by `deterministic_byte_for_byte_output_across_runs`)

## 7. Tail (mandatory)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/12-pre-thinking-injection.md` flipped to 🟢 Implemented; `docs/specs/00-index.md` row updated to 🟢
- [x] 7.2 Write tests covering the new behavior — `tests/pipeline.rs` (9) covers happy-path bundle with `query_id`, intent routing for `decision_lookup`, empty-response → empty-bundle counter bump, timeout → empty-bundle counter bump, `QueryFn` returning `None` flagged as fail-open, deterministic byte-for-byte output across runs, 80-KB-shaped overflow clipped under a 4-KB budget while keeping `LAW-007`, recent files within 5 min reaching `Scope.files`, truncation step metric recording each applied step. Lib unit tests (24) cover regex-bounded prompt-file extraction, semver-token filtering, sixteen-mention cap, repo resolution from `.git/` ancestor + `cortex.toml` override, recent-file age cut-off + dedupe, every keyword rule, fallback intent, `Intent::DecisionLookup` winning over refactor when both signals appear, fixed section order, empty-response shortcut, graph-section toggle, deterministic format output, UTF-8-safe `clip_utf8`, slim-snippet 3-line render, fits-within-budget short-circuit, 80-KB → ≤32-KB ladder run, step-order assertion, laws-never-dropped invariant
- [x] 7.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-pre-thinking --all-targets -- -D warnings`, `cargo test -p cortex-pre-thinking` all green (33 tests)
