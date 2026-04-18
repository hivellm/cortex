## 1. Module scaffold
- [ ] 1.1 `cortex-adapters/common/pre_thinking.rs` with `PreThinkingInput` + `PreThinkingBudget` + entry function
- [ ] 1.2 Re-export from `cortex-adapters/common/lib.rs` for per-tool adapters
- [ ] 1.3 Public error type + metrics hooks

## 2. Scope derivation
- [ ] 2.1 Repo resolution via nearest `.git/` ancestor + `cortex.toml` override
- [ ] 2.2 Files from `recent_files` (age <5 min) + verbatim prompt mentions (regex-bounded to 16 candidates)
- [ ] 2.3 Topic filtering left empty in v1 per spec 12 Decision 6

## 3. Intent selection
- [ ] 3.1 Keyword rule table (refactor → pre_change_context, why → decision_lookup, stuck → similar_problems, allowed → law_check, fallback → pre_change_context)
- [ ] 3.2 Unit tests per rule pair

## 4. Bundle formatter
- [ ] 4.1 Fixed section order: laws → decisions → similar turns → snippets → optional graph neighbors
- [ ] 4.2 Deterministic pure-Rust string assembly (no template engine)
- [ ] 4.3 Leading comment with `query_id` for audit correlation; trailing `<!-- end cortex -->`
- [ ] 4.4 Empty-response → empty string (no "nothing found" placeholder)

## 5. Budget clipper
- [ ] 5.1 Section caps: laws 10, decisions 5, similar turns 5, snippets 5, graph 0 default
- [ ] 5.2 6-step trim ladder (drop graph → trim snippets → halve snippets → halve turns → truncate decisions → drop snippets)
- [ ] 5.3 Laws section is invariant — never trimmed

## 6. Failure + audit
- [ ] 6.1 Fail-open: empty string on timeout / 5xx / formatter panic
- [ ] 6.2 Counter + span emission per spec 12 §Observability
- [ ] 6.3 Determinism property: identical input → byte-identical output

## 7. Tail (mandatory)
- [ ] 7.1 Update `docs/specs/12-pre-thinking-injection.md` status flag to 🟢 + index row
- [ ] 7.2 Integration tests: scope derivation fixtures; intent mapping per rule; 80 KB response clipped to ≤32 KB with documented step order; laws preserved; empty-response → empty string; 800 ms forced timeout → empty string + counter; deterministic golden bundle byte-match; snippet text-trim invariant
- [ ] 7.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
