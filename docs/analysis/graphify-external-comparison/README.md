# Graphify (safishamsi/graphify) — external comparison & what Cortex can borrow

> **Source:** https://github.com/safishamsi/graphify (active branch `v8`,
> PyPI `graphifyy`). YC S26. A Python library + AI-assistant skill that maps
> a codebase + docs + media into a queryable knowledge graph
> (`graph.json` / `graph.html` / `GRAPH_REPORT.md`).
> **Date:** 2026-06-20.
> **Scope:** read-only comparison. No Cortex source was modified.

## Why this analysis exists

Cortex and graphify solve overlapping problems from opposite directions:

- **Cortex** is a *capture-first* cognitive substrate. The graph
  (Nexus/Cypher) is one lane of a hybrid retrieval engine fed by live
  agent sessions, classifiers, embedders, and a governance layer. Its
  graph schema is event-centric (Session→Turn→ToolCall→Artifact→Symbol)
  with a static code/doc-correlation layer added in `phase11k`
  (`crates/cortex-workers/src/graph/{analyzer,markdown,resolver}/`).
- **Graphify** is a *snapshot-first* knowledge-graph builder. It treats
  the codebase + docs + PDFs + images + video as a corpus, extracts a
  graph deterministically (tree-sitter, 36 grammars) plus an optional
  LLM semantic pass, then ships three artifacts a human or agent can
  open and query — with **token-reduction** as the headline metric
  (claimed 71.5× fewer tokens/query on a 52-file mixed corpus).

Graphify is shipping several things Cortex's graph layer does *not*
have, and several of them are directly transplantable because Cortex
already runs tree-sitter and already has the static extractors that
feed Nexus. This is the "borrow the good parts" list.

## The numbered findings (read in order)

| # | File | Topic |
|---|------|-------|
| 00 | [`00-index.md`](./00-index.md) | This index + reading order |
| 01 | [`01-graphify-architecture.md`](./01-graphify-architecture.md) | What graphify is, its pipeline, and its output contract |
| 02 | [`02-cortex-vs-graphify.md`](./02-cortex-vs-graphify.md) | Side-by-side capability matrix |
| 03 | [`03-findings.md`](./03-findings.md) | Enumerated findings F-001..F-013 (evidence + impact + confidence) |
| 04 | [`04-language-coverage.md`](./04-language-coverage.md) | 4 analyzer languages vs 36 grammars — coverage gap |
| 05 | [`05-graph-analytics.md`](./05-graph-analytics.md) | Community detection, god nodes, confidence scoring |
| 06 | [`06-export-and-visualization.md`](./06-export-and-visualization.md) | callflow-HTML, Mermaid, wiki, Obsidian, GraphML, SVG |
| 07 | [`07-token-economics.md`](./07-token-economics.md) | The token-reduction benchmark and why it matters for pre-thinking |
| 08 | [`08-non-code-corpora.md`](./08-non-code-corpora.md) | PDFs, images, video, SQL/Postgres, Terraform, MCP configs |
| 09 | [`09-pr-intelligence.md`](./09-pr-intelligence.md) | `graphify prs` — graph-aware PR triage / conflict detection |
| 10 | [`10-execution-plan.md`](./10-execution-plan.md) | Phased plan to fold the worthwhile pieces into Cortex |

## Executive summary

Cortex's graph layer is **architecturally ahead** of graphify on the
parts that matter for a governed, live-capture substrate: bitemporal
schema, idempotent re-emit under `content_hash`, provenance
(`source_event_id`) on every edge, a real graph DB (Nexus) instead of
a NetworkX dump, and a hybrid retrieval fusion that graphify does not
attempt. Graphify has **no governance, no live capture, no bitemporal
history, no embeddings/vector lane** — it is a batch snapshotter.

But graphify is **ahead on five concrete, borrowable things**:

1. **Language breadth** — 36 tree-sitter grammars + regex extractors
   (Apex, Terraform/HCL, SQL/Postgres introspection, MCP-config and
   package-manifest extractors) vs Cortex's **4** analyzer languages
   (rust/python/typescript/go in
   `crates/cortex-workers/src/graph/analyzer/`). The grammars Cortex
   already vendors for the *embedder* (java/c/cpp) are not wired into
   the *graph analyzer* yet. **F-001, F-002.**
2. **Graph analytics on top of the raw graph** — Leiden community
   detection, "god node" centrality ranking, and "surprising
   connection" scoring. Cortex stores edges but computes none of
   these summary structures. **F-005, F-006.**
3. **Discrete confidence rubric** — every inferred edge gets
   `EXTRACTED | INFERRED | AMBIGUOUS` plus a 0.55–0.95 numeric score
   with a documented rubric. Cortex's static edges carry a
   `confidence` prop but no published rubric or `AMBIGUOUS` triage
   surface. **F-007.**
4. **Human/agent-facing graph artifacts** — `callflow-html` (Mermaid
   architecture diagrams), `--wiki`, `--obsidian`, `--svg`,
   `--graphml`, and an interactive `graph.html`. Cortex's only graph
   surface is the dashboard graph view + raw Cypher. **F-008, F-009.**
5. **Token-reduction framing** — graphify measures and advertises
   "tokens per query vs reading raw files". Cortex's pre-thinking
   bundle is exactly this value proposition but is **not measured**
   against a raw-file baseline. Adopting the benchmark would give
   Cortex a defensible quality metric. **F-010.**

Plus two adjacent ideas worth tracking but lower priority:
graph-aware **PR triage / merge-conflict prediction** (`graphify prs`,
**F-012**) and **non-code corpora** ingestion — PDFs/images/video
(**F-011**).

## Bottom line recommendation

Do **not** adopt graphify as a dependency or restructure Cortex toward
its batch model. Cortex's live-capture + bitemporal + governance
architecture is the right one. Instead, **harvest five capabilities**
into the existing `cortex-workers` graph stack, in priority order:

1. **Widen the analyzer to the grammars already vendored** (java, c,
   cpp) — near-zero new deps, closes the most damaging coverage gap.
   (Phase 1.)
2. **Add a community-detection + god-node pass** over the Nexus graph,
   surfaced in pre-thinking and the dashboard. (Phase 2.)
3. **Publish a confidence rubric + an `AMBIGUOUS` triage view** for
   static edges. (Phase 2.)
4. **Add a token-reduction eval** to `cortex-eval` measuring bundle
   bytes vs raw-file bytes per query intent. (Phase 1, cheap.)
5. **Add a Mermaid/callflow export** from the graph for human
   architecture review. (Phase 3.)

See [`10-execution-plan.md`](./10-execution-plan.md) for the phased,
Rulebook-task-mappable plan.
