## 1. Intent enum + selector
- [ ] 1.1 In the `Intent` enum (`crates/cortex-api/src/types.rs` or wherever it lives), add `Explain` variant; serialise as `"explain"`
- [ ] 1.2 In `crates/cortex-pre-thinking/src/intent_select.rs`, add the keyword table for `Intent::Explain` per §What Changes (priority-ordered substring match)
- [ ] 1.3 Extend the existing keyword tables (`DecisionLookup`, `SimilarProblems`, `LawCheck`) with the missing common phrasings listed in the proposal
- [ ] 1.4 Make the matcher return both the matched `Intent` AND the trigger keyword (e.g. `MatchedIntent { intent: Intent, trigger: &'static str }`) so the audit envelope can record which keyword fired

## 2. Plan factory
- [ ] 2.1 Locate the per-intent plan factories in `crates/cortex-pre-thinking/src/strategies.rs` (or `crates/cortex-api/src/strategies.rs` if that is where `Intent` plans live)
- [ ] 2.2 Add `explain_plan(req: &QueryRequest) -> Plan` returning vector + keyword fan-out on `topics = ["code", "docs"]`, `limit = 8` per lane
- [ ] 2.3 Overlays: `IncludeField::Snippets` only — no `Decisions`, `Violations`, `SimilarTurns`
- [ ] 2.4 Graph leg: when `phase4c` has shipped (detected via `edge_artifact_definitions` strategy existing in the strategies module), include it; otherwise no graph leg
- [ ] 2.5 Wire `Intent::Explain` into the `build_plan` dispatch table

## 3. Audit envelope
- [ ] 3.1 Extend `AuditEnvelope` with `intent_trigger: Option<String>` (the matched keyword); stamp from the selector outcome
- [ ] 3.2 Update the audit fixture in `crates/cortex-api/tests/http.rs` to assert `intent` + `intent_trigger` are present on responses driven by an explanatory prompt

## 4. Tests
- [ ] 4.1 In `intent_select.rs::tests`, add cases for every new keyword across the four extended intents — assert each routes to the expected variant
- [ ] 4.2 Assert "no keyword matches" still falls through to `Intent::PreChangeContext` (regression invariant)
- [ ] 4.3 Add a plan-factory test asserting `Intent::Explain` plans have empty `decisions` / `violations` / `similar_turns` overlays

## 5. End-to-end test
- [ ] 5.1 In `crates/cortex-pre-thinking/tests/` (or `cortex-api/tests/http.rs`), drive `/v1/query` with a prompt like `"explain how the meili fan-out works"` and assert the response carries `intent = "explain"` + non-empty snippets + empty decisions/violations/similar_turns

## 6. Spec docs
- [ ] 6.1 In `docs/specs/12-pre-thinking.md`, document the `Intent::Explain` variant (when it fires, what plan it produces) and the expanded keyword tables
- [ ] 6.2 Cross-link from `docs/analysis/relevance/01-findings.md` §F-006 (mark closed-by phase6d on merge)

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation — `docs/specs/12-pre-thinking.md` per §6
- [ ] 7.2 Write tests covering the new behavior — keyword routing tests in §4 plus the end-to-end test in §5
- [ ] 7.3 Run tests and confirm they pass — `cargo clippy -p cortex-api -p cortex-pre-thinking --all-targets -- -D warnings` and `cargo test -p cortex-api -p cortex-pre-thinking` both green
