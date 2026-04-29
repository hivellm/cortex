## 1. Intent enum + selector
- [x] 1.1 In the `Intent` enum (`crates/cortex-api/src/types.rs`), added `Explain` variant; `Intent::Explain.label() = "explain"` (snake-case via existing `serde(rename_all = "snake_case")`)
- [x] 1.2 In `crates/cortex-pre-thinking/src/intent_select.rs`, added the priority-ordered keyword table for `Intent::Explain`: `how does`, `what is`, `what's`, `explain`, `show me`, `where is`, `where does`, `find usages`, `find references`, `look up`, `definition of` — placed FIRST so a prompt mixing `explain` + `why` routes to the navigational intent rather than burning the decisions overlay
- [x] 1.3 Extended `decision_lookup` (`why did we pick`, `why do we use`, `history of`), `similar_problems` (`have we seen`, `did we hit`), and `law_check` (`is this allowed`, `am i allowed`, `would this violate`) tables per the proposal
- [x] 1.4 Selector now returns `MatchedIntent { intent: Intent, trigger: Option<&'static str> }` via the new `select_matched` / `select_matched_with` entry points; legacy `select` / `select_with` remain as wrappers that drop the trigger so existing callers don't break

## 2. Plan factory
- [x] 2.1 Located the per-intent plan factories in `crates/cortex-api/src/strategies.rs` (where `Intent` plans live)
- [x] 2.2 Added `fn explain(req: &QueryRequest) -> Plan` returning vector + keyword fan-out on `code` + `docs` collections, `k` / `limit` capped at 8 per lane
- [x] 2.3 Overlays array is empty (`overlays_from_include(&req.include, &[])`) so even when the caller's `include` array asks for decisions / violations / similar_turns, the Explain plan strips them
- [x] 2.4 Graph leg: empty `Vec::new()` until `phase4c` ships `edge_artifact_definitions`; comment in `explain()` flags the dependency on the unblocking phase (the `phase4c` task is the gate, not a deferral inside phase6d)
- [x] 2.5 Wired `Intent::Explain => explain(req)` into the `build_plan` dispatch table

## 3. Audit envelope
- [x] 3.1 Phase6d added `intent_trigger: Option<&str>` to `build_envelope_with_audit_context` in `crates/cortex-api/src/audit.rs`; serialised as `Value::String(...)` when set, explicit `Value::Null` when absent so dashboards can tell "no rule fired" apart from "old daemon didn't record the field". Service reads the new `x-cortex-intent-trigger` HTTP header (constant `HEADER_CORTEX_INTENT_TRIGGER`) and threads the value through both audit emit sites (cache hit + cache miss). The adapter (`sync_paths.rs`) re-derives the trigger off the prompt via `select_matched` and sets the header on the outbound `/v1/query` POST
- [x] 3.2 `audit_publisher_emits_one_envelope_per_request` extended to assert `intent_trigger == Null` on the in-process path; new `audit_envelope_carries_intent_trigger_from_header` test pins the header → envelope round-trip on the HTTP path

## 4. Tests
- [x] 4.1 In `intent_select.rs::tests`, added 19 cases covering every new keyword across `Explain` (11 phrases), extended `DecisionLookup` (3), extended `SimilarProblems` (2), and extended `LawCheck` (3); each asserts both the resolved `Intent` and (where pinned) the matched trigger string
- [x] 4.2 `fallback_is_pre_change_context` retained; new `fallback_carries_no_trigger` asserts `MatchedIntent { intent: Intent::PreChangeContext, trigger: None }` on a no-keyword prompt
- [x] 4.3 In `strategies.rs::tests`, three new cases — `explain_uses_vector_and_keyword_no_graph_no_overlays`, `explain_caps_per_lane_fan_out_at_eight`, `explain_preserves_smaller_caller_caps` — pin the plan's vector/keyword shape, the no-overlay invariant, and the k/limit cap behaviour

## 5. End-to-end test
- [x] 5.1 In `crates/cortex-api/tests/http.rs`, `explain_intent_returns_snippets_with_no_overlay_noise` drives `/v1/query` with `intent = explain` against a seed that carries `decision_id` extras; asserts the response carries `intent: "explain"`, non-empty snippets, and EMPTY decisions / violations / similar_turns / graph_neighbors arrays even though the lane projection contract WOULD surface them under any other intent — the strip is the win condition for F-006

## 6. Spec docs
- [x] 6.1 Rewrote `docs/specs/12-pre-thinking-injection.md` §intent selection: full keyword table per intent (with priority ordering note explaining why `Explain` evaluates first), per-intent plan summary, `MatchedIntent` shape, audit envelope `intent_trigger` field
- [x] 6.2 `docs/analysis/relevance/01-findings.md` §F-006 "Tracked by" line now points at `phase6d_relevance_intent_table_expansion` + the 11-keyword `Explain` table + the spec section

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/12-pre-thinking-injection.md` §intent selection per §6 + closure note in `docs/analysis/relevance/01-findings.md` §F-006
- [x] 7.2 Write tests covering the new behavior — 19 keyword routing tests + 1 trigger-fallback case in `intent_select.rs`; 3 plan-factory cases in `strategies.rs`; 2 e2e cases in `tests/http.rs`
- [x] 7.3 Run tests and confirm they pass — `cargo test -p cortex-api -p cortex-pre-thinking --lib --tests` green: cortex-api 136 lib + 24 http + 5 + 3 + 6 + 6 = 180; cortex-pre-thinking 44 lib + 9 + 40 = 93. `cargo clippy -p cortex-api -p cortex-pre-thinking --lib --tests` shows the same 7 pre-existing `dashboard.rs` warnings the prior phase6 tails noted; zero warnings on phase6d-touched files.
