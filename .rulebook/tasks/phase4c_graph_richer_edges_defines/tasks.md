## 1. Schema additions
- [ ] 1.1 Add `Symbol` node label and `DEFINES` edge type to `cortex-graph::schema`
- [ ] 1.2 `Symbol` natural key in `identity.rs`: `(repo, language, qualified_name)` — fallback to `(repo, path, name)` when no FQN
- [ ] 1.3 Property defaults: `name` (string, required), `language` (string, required), `signature` (string, optional), `kind` (string: `function|struct|class|trait|enum|other`)

## 2. Mapper — emit symbol patches
- [ ] 2.1 In `mapper.rs`, for every artifact-chunk event with `source == "code"` and a non-empty `symbol` field, produce a `Patch::MergeNode(Symbol)` and a `Patch::MergeEdge(Symbol -> Artifact, DEFINES)`
- [ ] 2.2 Tolerate events whose `symbol` is missing — they become Artifact-only patches as before; no error
- [ ] 2.3 Coalesce duplicate symbols within a single batch — already handled by `coalescer.rs`; assert the test path covers it

## 3. Cypher templates
- [ ] 3.1 `render_symbol_merge` produces `MERGE (s:Symbol {repo, language, qname}) ON CREATE SET ...` honoring the project's identity convention
- [ ] 3.2 `render_defines_merge` produces `MATCH (s:Symbol ...), (a:Artifact ...) MERGE (s)-[:DEFINES]->(a)` with the same MATCH-fail-tolerance the existing edge MERGEs use (silent drop logged at debug)
- [ ] 3.3 Register both templates in `CypherTemplates`

## 4. Backfill the existing graph
- [ ] 4.1 Replay the event archive once via the updated `cortex-graph` worker — Nexus `MERGE` makes this idempotent
- [ ] 4.2 After replay, assert via Cypher: `MATCH (s:Symbol)-[:DEFINES]->(a:Artifact) RETURN count(s) AS sym, count(DISTINCT a) AS art` returns non-zero on both columns
- [ ] 4.3 Spot-check: query for `PreThinkingTool` returns `crates/cortex-mcp-server/src/tools.rs` in the `Cortex` repo

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation (extend spec-12 with a `## Symbols & DEFINES` section)
- [ ] 5.2 Write tests covering the new behavior (mapper unit test, Cypher rendering snapshot, end-to-end against a temp Nexus container with a 3-event fixture archive)
- [ ] 5.3 Run tests and confirm they pass (`cargo check -p cortex-graph` → `cargo clippy -p cortex-graph --all-targets -- -D warnings` → `cargo test -p cortex-graph`)
