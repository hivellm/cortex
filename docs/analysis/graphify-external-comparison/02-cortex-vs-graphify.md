# 02 — Cortex vs Graphify: capability matrix

Legend: ✅ has it · ⚠️ partial · ❌ absent.

## Where Cortex is ahead (do not regress these)

| Capability | Cortex | Graphify | Notes |
|---|:--:|:--:|---|
| Live agent-session capture | ✅ | ❌ | Session→Turn→ToolCall→Artifact event model (`crates/cortex-core`, adapters) |
| Bitemporal history | ✅ | ❌ | spec 30, `crates/cortex-workers/src/graph/bitemporal.rs` |
| Real graph DB (Nexus/Cypher) | ✅ | ❌ | graphify is a NetworkX JSON dump |
| Hybrid retrieval fusion (vector+keyword+graph, RRF) | ✅ | ❌ | spec 11, `cortex-api` fusion |
| Vector/embedding lane | ✅ | ❌ | graphify uses graph structure as the similarity signal |
| Governance / laws / trust | ✅ | ❌ | specs 13–14, `cortex-laws` |
| Provenance on every edge (`source_event_id`) | ✅ | ⚠️ | graphify has `source_file` per edge but no event lineage |
| Idempotent re-emit under `content_hash` | ✅ | ⚠️ | graphify uses SHA256 file cache; no edge-identity guarantee |
| Branches / timeline | ✅ | ❌ | specs 32–33, `branch.rs` |
| Cross-project axis | ✅ | ⚠️ | Cortex spec 34; graphify has `global` graph (flatter) |

## Where Graphify is ahead (the borrow list)

| Capability | Cortex | Graphify | Finding |
|---|:--:|:--:|---|
| Tree-sitter language breadth (graph analyzer) | ⚠️ 4 | ✅ 36 | [F-001](./03-findings.md) |
| Grammars vendored but not wired to analyzer | ⚠️ | — | [F-002](./03-findings.md) |
| Non-tree-sitter extractors (Apex, Terraform, SQL, MCP cfg, manifests) | ❌ | ✅ | [F-003](./03-findings.md) |
| Live DB-schema introspection (Postgres/Cargo) | ❌ | ✅ | [F-004](./03-findings.md) |
| Community detection (Leiden) | ❌ | ✅ | [F-005](./03-findings.md) |
| God-node centrality ranking | ❌ | ✅ | [F-006](./03-findings.md) |
| Confidence rubric + AMBIGUOUS triage | ⚠️ | ✅ | [F-007](./03-findings.md) |
| Mermaid / callflow architecture export | ❌ | ✅ | [F-008](./03-findings.md) |
| Wiki / Obsidian / SVG / GraphML exports | ❌ | ✅ | [F-009](./03-findings.md) |
| Token-reduction benchmark (vs raw files) | ❌ | ✅ | [F-010](./03-findings.md) |
| Non-code corpora (PDF/image/video) | ❌ | ✅ | [F-011](./03-findings.md) |
| Graph-aware PR triage / conflict prediction | ❌ | ✅ | [F-012](./03-findings.md) |
| "Surprising connection" scoring | ❌ | ✅ | [F-013](./03-findings.md) |

## Reading the matrix

The two halves are almost disjoint, which is the key insight: graphify
is **not a competitor to Cortex's architecture** — it is a *batch graph
toolkit* whose analytics + export + breadth layers sit on top of
exactly the kind of static-extraction graph Cortex already builds in
`crates/cortex-workers/src/graph/`. Everything in the second table is
additive to Cortex without touching its event model, governance, or
fusion engine.

The highest-leverage borrows are the ones that reuse machinery Cortex
already has:

- **F-002** (wire vendored grammars) is nearly free — the deps are in
  `crates/cortex-workers/Cargo.toml` already.
- **F-005/F-006** (community + god nodes) are graph-algorithm passes
  over Nexus output; no new extraction.
- **F-010** (token benchmark) is an eval, not a feature — it slots into
  `crates/cortex-eval`.
