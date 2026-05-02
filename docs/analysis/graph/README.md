# Cortex graph layer — gap analysis + correlation plan

> **Trigger:** *"a parte de graph ta uma merda, nao existe corelacoes basicas
> das documentacoes com o os codigos, nem mesmo imports de modulos entro dos
> codigos, isso deixa o conteudo quase inutil"*

Files in this directory:

1. [`01-current-state.md`](./01-current-state.md) — what nodes / edges
   exist today, end to end (mapper.rs, classifier-extracted entities,
   schema bootstrap).
2. [`02-gaps.md`](./02-gaps.md) — what's missing and why each missing
   edge class kills relevance for a concrete query intent.
3. [`03-target-graph.md`](./03-target-graph.md) — proposed node + edge
   schema with the new code-structure / doc↔code / cross-doc layers.
4. [`04-extraction-pipeline.md`](./04-extraction-pipeline.md) — how to
   actually compute every new edge (Tree-sitter queries, markdown
   parser, path/symbol resolver, idempotency under content_hash).
5. [`05-impact-and-risks.md`](./05-impact-and-risks.md) — quantitative
   estimate of graph density lift, query bundle impact, false-positive
   risks, grammar coverage holes.
6. [`06-implementation-plan.md`](./06-implementation-plan.md) — phased
   roadmap, mapped onto a Rulebook task tree (`phase11k_graph_correlations`).

## TL;DR

The graph layer ships a **structural skeleton** (Session→Turn→ToolCall→
Artifact, Artifact↔Symbol via DEFINES) plus a **best-effort Sonnet
semantic layer** (REFERENCES / IMPLEMENTS / FIXES / DISCUSSES /
DEPENDS_ON / SUPERSEDES) extracted per event from classifier output.

What is **completely absent**:

| Layer                              | Missing edges                              |
| ---------------------------------- | ------------------------------------------ |
| Code↔code (intra-file + intra-repo) | `IMPORTS`, `CALLS`, `USES_TYPE`, `EXTENDS` |
| Code↔code (cross-repo)              | `IMPORTS` resolved against external SDK    |
| Doc→code                            | `MENTIONS` (doc mentions a symbol/path)    |
| Doc→code (rich)                     | `DOCUMENTS` (file→file Markdown link)      |
| Code→doc                            | `DOCUMENTED_BY` (Rust doc-comment links)   |
| Doc↔doc                             | `LINKS_TO` (Markdown `[label](other.md)`)  |
| Decision↔doc                        | `CITES` (ADR cites spec/analysis/learning) |
| Spec↔spec                           | `REFERENCES` (`docs/specs/12.md` cites 11) |

Without these, every code-only or doc-anchored query — *"what calls
`hnsw_search`?"*, *"which spec covers fusion?"*, *"who imports
`vectorizer_sdk`?"* — bottoms out. The graph is decorative, not
load-bearing.

The Sonnet semantic layer is **expensive** (per-event classification
cost) and **lossy** (only fires when a turn happens to discuss an
entity). Static extraction (Tree-sitter + Markdown link parser) is
free at write time and produces orders of magnitude more edges
deterministically.

## Headline numbers (all estimates against current corpus)

| Metric                                          | Today              | Target             | Lift     |
| ----------------------------------------------- | ------------------ | ------------------ | -------- |
| Edges per Artifact (code, avg)                  | 2 (IN_REPO, DEFINES) | ~12 (+ IMPORTS, CALLS, USES_TYPE) | **6×**   |
| Edges per doc Artifact (avg)                    | 1 (IN_REPO)        | ~8 (+ MENTIONS×N)  | **8×**   |
| `pre_change_context` 2-hop hit rate (estimate)  | ~28 %              | ~75 %              | **2.7×** |
| `decision_lookup` doc-trail completeness        | ~10 %              | ~80 %              | **8×**   |
| Cross-repo symbol resolution                    | 0 %                | ~60 % (top SDKs)   | ∞        |

Static extraction adds ~0.3 ms per file at bootstrap time (Tree-sitter
already runs for symbol chunking; we add 3-4 query passes). Markdown
extraction is ~0.1 ms per file. Net cost vs Sonnet semantic layer:
roughly 1/100th per edge, deterministic, idempotent under content_hash.

## How to read this

Start at [`02-gaps.md`](./02-gaps.md) for the failure-mode walkthrough
(why each missing edge breaks a real query). [`03-target-graph.md`](./03-target-graph.md)
defines the new schema. [`04-extraction-pipeline.md`](./04-extraction-pipeline.md)
covers implementation. [`06-implementation-plan.md`](./06-implementation-plan.md)
phases the rollout into a Rulebook task tree.

## Crate placement

**No new crates.** Per the project's standing rule (consolidate into
existing workspace members instead of fanning out into more
sub-services), every new module lands inside an existing crate:

- Static analyzers + Markdown parser + symbol resolver →
  `crates/cortex-workers/src/graph/{analyzer,markdown,resolver}/`
  (sibling to the existing `mapper.rs` / `worker.rs`).
- Cross-repo SDK declarations →
  `crates/cortex-storage/src/external_repos.rs`.
- Bootstrap flag → `crates/cortex-cli/src/bin/cortex-bootstrap.rs`
  (existing binary, new `--graph-static` flag).
- Renderer integration →
  `crates/cortex-pre-thinking/src/formatter.rs` (existing crate,
  new sub-blocks).
- Cypher templates → `crates/cortex-workers/cypher/` (existing
  registry).

The only new workspace dep is `pulldown-cmark` (added to
`cortex-workers/Cargo.toml`).
