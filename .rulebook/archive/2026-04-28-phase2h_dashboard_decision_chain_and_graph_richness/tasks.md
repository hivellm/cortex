## 1. Backend — decisions chain + cites
- [x] 1.1 Extend `DecisionRow` with `chain: Vec<ChainNode>` and `cites_7d: u64`
- [x] 1.2 `ChainNode = { id: String, title: String, state: "current" | "old" }`
- [x] 1.3 Build the chain by walking `supersedes` backwards from the current decision until a node with no `supersedes` is reached; current decision marked `"current"`, ancestors marked `"old"`
- [x] 1.4 `cites_7d` derived by counting envelopes captured in the last 7 days whose body matches the regex `\bDEC-\d{4}-\d{3}\b` and resolves to this decision id
- [x] 1.5 Empty `chain` (single-node) is `[]` not `[{state: "current", ...}]` — keeps the renderer logic simple

## 2. Backend — decision detail endpoint
- [x] 2.1 Add `/v1/dashboard/decisions/{id}` route returning `{ ...DecisionRow, body_markdown: String }`
- [x] 2.2 `body_markdown` is the full envelope body (not the 600-char clip the list returns)
- [x] 2.3 404 with `{ "reason": "decision_not_found" }` when no envelope matches

## 3. Backend — graph node expansion
- [x] 3.1 Iterate captured envelopes and emit one node per: Session (1), Turns, ToolCalls, Decisions, Laws (synthesized when a violation references a `law_id`), Violations, Analyses, Artifacts (distinct file paths from Edit/Write tool inputs) — superseded by spec-07 `cortex-graph::mapper`, which emits all eight kinds direct to Nexus; the dashboard endpoint reads them via `query_nexus_graph` (edge-first)
- [x] 3.3 Edge types: `CONTAINS` (Session→Turn), `INVOKED` (Turn→ToolCall), `WROTE`/`READ` (ToolCall→Artifact), `REFERENCES` (Artifact→Decision when the artifact path matches `decisions/<id>.md`), `OBSERVED_IN` (Violation→ToolCall), `OF` (Violation→Law), `PRODUCED` (Analysis→Decision when the analysis body cites the decision id) — superseded by spec-07's canonical names: `HAS_TURN`, `HAS_TOOL_CALL`, `TOUCHED`, `OBSERVED_IN`, `OF`, `SUPERSEDES`, `LINKED_TO`, `IN_REPO`, `REMEMBERS`. Same semantics, names locked to the architecture §4.2 schema
- [x] 3.2 Cap total nodes at 60; surface `truncated: true` in the response when exceeded — abandoned-by-design: the dashboard caps at 50,000 (raised to fit the full panorama) and the response does NOT carry a `truncated` flag. The frontend tracks "shown / hidden leaf" by post-filtering `data.nodes` against the structural-skeleton kind set; replaced by the GUI side-panel counter
- [x] 3.4 Layout: keep the existing column-based x coordinates from the current handler; y coordinates spread evenly within each column (no explicit force layout) — abandoned-by-design: column layout dropped in favour of ForceAtlas2 (Sigma + graphology) because column placements did not scale past 60 nodes and the Sigma WebGL renderer handles 30k+ natively
- [x] 3.5 Update `GraphPayload` shape: `{ nodes, edges, truncated: bool }` — abandoned-by-design: the structural-skeleton filter on the frontend tracks `shown / hidden` from `data.nodes` directly; no backend flag needed

## 4. Frontend — type updates + render adjustments
- [x] 4.1 `gui/src/lib/api.ts`: extend `DecisionRow` (`chain`, `cites_7d`), add `DecisionDetail` type, extend `GraphPayload` (`truncated`) — `chain` / `cites_7d` / `DecisionDetail` landed; `truncated` abandoned as in 3.5
- [x] 4.2 Decisions view picks up `chain` automatically (rendered by phase2d)
- [x] 4.3 Decisions stats grid gains a "Cited 7d" tile from `sum(cites_7d)` once values land
- [x] 4.4 Graph view shows a `truncated: showing first 60 of N nodes` banner when the flag is true — replaced by the side-panel `shown / edges / hidden N leaf` row in Graph.tsx (drives off the structural-skeleton filter result)
- [x] 4.5 Graph legend already covers all node kinds — no change needed there

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `docs/specs/16-dashboard.md` adds shape blocks for `/decisions` (chain + cites_7d), `/decisions/{id}` (detail + body_markdown), and `/graph` (live spec-07 vs synthetic fallback with edge schema). `gui/README.md` Decisions row updated to describe chain rendering + Cited 7d tile + detail drawer; Graph row rewritten around Sigma WebGL + ForceAtlas2 + side-panel `shown/edges/hidden N leaf` (the `truncated` banner was abandoned in 3.5/4.4)
- [x] 5.2 Write tests covering the new behavior — added 6 backend tests in `dashboard.rs`: `decisions_walk_supersedes_chain_oldest_first` (3-link chain ordering + reverse pointer + status flip), `decisions_emit_empty_chain_for_solitary_decision`, `decisions_count_cites_in_last_7d_and_skip_self_envelope`, `decision_detail_returns_full_markdown_body`, `decision_detail_404s_on_unknown_id`, `graph_synthesises_decision_analysis_violation_nodes_from_lane`. The `truncated:true past 60 nodes` test from the original plan is intentionally NOT shipped — that path was abandoned per 3.5
- [x] 5.3 Run tests and confirm they pass — `cargo test -p cortex-api` → 143 pass (109 lib + 6 + 14 + 5 + 3 + 6, 0 fail). `cargo clippy -p cortex-api --all-targets -- -D warnings` → 7 baseline errors remain in pre-existing dashboard sections (zero introduced by phase2h). `pnpm exec tsc --noEmit -p tsconfig.json` clean
