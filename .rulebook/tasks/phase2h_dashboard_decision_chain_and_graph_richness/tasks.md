## 1. Backend — decisions chain + cites
- [ ] 1.1 Extend `DecisionRow` with `chain: Vec<ChainNode>` and `cites_7d: u64`
- [ ] 1.2 `ChainNode = { id: String, title: String, state: "current" | "old" }`
- [ ] 1.3 Build the chain by walking `supersedes` backwards from the current decision until a node with no `supersedes` is reached; current decision marked `"current"`, ancestors marked `"old"`
- [ ] 1.4 `cites_7d` derived by counting envelopes captured in the last 7 days whose body matches the regex `\bDEC-\d{4}-\d{3}\b` and resolves to this decision id
- [ ] 1.5 Empty `chain` (single-node) is `[]` not `[{state: "current", ...}]` — keeps the renderer logic simple

## 2. Backend — decision detail endpoint
- [ ] 2.1 Add `/v1/dashboard/decisions/{id}` route returning `{ ...DecisionRow, body_markdown: String }`
- [ ] 2.2 `body_markdown` is the full envelope body (not the 600-char clip the list returns)
- [ ] 2.3 404 with `{ "reason": "decision_not_found" }` when no envelope matches

## 3. Backend — graph node expansion
- [ ] 3.1 Iterate captured envelopes and emit one node per: Session (1), Turns, ToolCalls, Decisions, Laws (synthesized when a violation references a `law_id`), Violations, Analyses, Artifacts (distinct file paths from Edit/Write tool inputs)
- [ ] 3.2 Cap total nodes at 60; surface `truncated: true` in the response when exceeded
- [ ] 3.3 Edge types: `CONTAINS` (Session→Turn), `INVOKED` (Turn→ToolCall), `WROTE`/`READ` (ToolCall→Artifact), `REFERENCES` (Artifact→Decision when the artifact path matches `decisions/<id>.md`), `OBSERVED_IN` (Violation→ToolCall), `OF` (Violation→Law), `PRODUCED` (Analysis→Decision when the analysis body cites the decision id)
- [ ] 3.4 Layout: keep the existing column-based x coordinates from the current handler; y coordinates spread evenly within each column (no explicit force layout)
- [ ] 3.5 Update `GraphPayload` shape: `{ nodes, edges, truncated: bool }`

## 4. Frontend — type updates + render adjustments
- [ ] 4.1 `gui/src/lib/api.ts`: extend `DecisionRow` (`chain`, `cites_7d`), add `DecisionDetail` type, extend `GraphPayload` (`truncated`)
- [ ] 4.2 Decisions view picks up `chain` automatically (rendered by phase2d)
- [ ] 4.3 Decisions stats grid gains a "Cited 7d" tile from `sum(cites_7d)` once values land
- [ ] 4.4 Graph view shows a `truncated: showing first 60 of N nodes` banner when the flag is true
- [ ] 4.5 Graph legend already covers all node kinds — no change needed there

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend `docs/specs/16-dashboard.md` with the new shapes; extend `gui/README.md` Decisions + Graph sub-sections with the chain rendering and truncation banner
- [ ] 5.2 Write tests covering the new behavior — Rust integration tests: decisions endpoint returns the chain (build a 3-decision supersession chain, assert ordering), `cites_7d` count matches a known seeded set, decision detail endpoint returns body_markdown and 404s on unknown id, graph endpoint emits Decision/Law/Violation/Analysis/Artifact nodes from a seeded lane, graph endpoint sets `truncated: true` past 60 nodes; RTL: Decisions chain renders when `chain.length > 1`, Graph banner appears when `truncated: true`
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `pnpm test`, `pnpm exec tsc --noEmit -p tsconfig.json`
