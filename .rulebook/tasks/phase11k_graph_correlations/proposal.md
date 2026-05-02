# Proposal: phase11k_graph_correlations

Source: [`docs/analysis/graph/`](../../../docs/analysis/graph/)

## Why

The Cortex graph layer today ships a **structural skeleton**
(Session→Turn→ToolCall→Artifact, Artifact↔Symbol via DEFINES) plus
an opportunistic Sonnet semantic layer. What's **completely absent**:

- **Code↔code edges**: no `IMPORTS`, no `CALLS`, no `USES_TYPE`. The
  Tree-sitter parser already runs (for symbol chunking inside
  `cortex-workers/src/embedder/chunker_code.rs`) but the
  use_declaration / call_expression / type_reference nodes are
  never visited.
- **Doc→code edges**: a doc mentioning `FusionConfig` or linking to
  `crates/cortex-api/src/fusion.rs` produces zero edges.
- **Code→doc backlinks**: Rust intra-doc `[`crate::Sym`]` references
  vanish.
- **Spec↔spec / ADR↔spec edges**: every markdown citation is text,
  never traversable.
- **Cross-repo references**: `use vectorizer_sdk::HnswSearch` is
  invisible to the graph.

Without these, every code-only or doc-anchored query — *"what calls
`hnsw_search`?"*, *"which spec covers fusion?"*, *"who imports
`vectorizer_sdk`?"* — bottoms out. The graph is decorative, not
load-bearing.

The Sonnet semantic layer is **expensive** (per-event classification
budget) and **lossy** (only fires on ~5-10 % of events with ~1-3
edges each). Static extraction (Tree-sitter + Markdown link parser)
is free at write time and produces orders of magnitude more edges
deterministically.

This phase adds three new edge classes — code-structure, doc↔code,
cross-doc — extracted statically, deterministically, idempotent
under content_hash. Estimated graph density lift: **10×** on total
edges. Estimated relevance lift on the phase11i gold-set: **+0.07
absolute MRR@10**. See
[`docs/analysis/graph/05-impact-and-risks.md`](../../../docs/analysis/graph/05-impact-and-risks.md)
for the full impact estimate + risk register.

This phase depends on `phase11i_claude_archive_indexer_and_relevance`
(the gold-set IT is the headline acceptance gate) and runs partly
parallel to `phase11j_consolidation_tier` (different files, no
crate overlap).

## What Changes

**No new crates.** All new modules land inside existing workspace
members so the dep tree stays tight (per the user's standing rule
that Cortex consolidates under the principal crates instead of
fanning out into more sub-services). The natural homes:

- **Static analyzers** → `crates/cortex-workers/src/graph/analyzer/`
  (sibling to `mapper.rs`). The Tree-sitter machinery already lives
  in this crate's `embedder/chunker_code.rs`; the analyzer reuses
  the same grammar set + adds three more queries per language.
- **Symbol / module / package resolver** →
  `crates/cortex-workers/src/graph/resolver/`. The crate already
  walks the workspace at boot for embedder routing; the resolver
  hangs off the same walker.
- **Markdown analyzer** →
  `crates/cortex-workers/src/graph/markdown.rs`. Pulldown-cmark dep
  on cortex-workers (already present transitively via tower-http).
- **Cross-repo SDK declarations** →
  `crates/cortex-storage/src/external_repos.rs` (TOML loader +
  resolver). Storage already owns naming conventions.
- **Cypher templates** → `crates/cortex-workers/cypher/` (existing
  registry).
- **Renderer integration** →
  `crates/cortex-pre-thinking/src/formatter.rs` (existing crate;
  new sub-blocks under `Past sessions` / `Consolidated context`).
- **Bootstrap flag** → `crates/cortex-cli/src/bin/cortex-bootstrap.rs`
  (existing binary; new `--graph-static` flag).
- **Live trigger** → `crates/cortex-workers/src/graph/worker.rs`
  (existing graph worker; new content-hash diff path).

### §1 — Tree-sitter analyzer + Rust support

`crates/cortex-workers/src/graph/analyzer/mod.rs` — `CodeAnalyzer`
trait + `CodeEdge` / `ResolutionTarget` types. `analyzer/rust.rs`
ships four Tree-sitter queries (use_decl, call_expr, type_use,
impl_block). Three-tier symbol resolver (local-file, crate-index,
external/unresolved) backed by a `ModuleMap` built once at
bootstrap from the workspace's Cargo manifests.

### §2 — TypeScript / Python / Go analyzers

`analyzer/typescript.rs`, `analyzer/python.rs`, `analyzer/go.rs`.
Same `CodeAnalyzer` impl, language-specific Tree-sitter queries.
Multi-language workspace IT proves cross-language coherence.

### §3 — Markdown analyzer + doc↔code edges

`crates/cortex-workers/src/graph/markdown.rs` with pulldown-cmark
walker emitting `:LINKS_TO`, `:DOCUMENTS`, `:LINKS_TO_SECTION`,
`:MENTIONS`, `:DESCRIBES_PATH`, `:DOCUMENTED_BY`. Three-tier
disambiguation for symbol mentions. Section-level extraction
producing `:DocSection` nodes so spec-12 §Output gets its own
edge target.

### §4 — Decision / Knowledge / Learning / Consolidation citations

Same markdown analyzer runs over `Decision` / `Knowledge` /
`Learning` / `Consolidation` payload bodies. `Decision.payload.links[]`
(currently dead in the graph layer) becomes typed `:CITES` edges.
`Consolidation.payload.source_event_ids[]` materialises as
`:DERIVED_FROM` edges so the curated layer is navigable.

### §5 — Bootstrap + live triggers

`cortex-bootstrap --graph-static` runs the static analyzers as
part of the workspace pass. `cortex-workers/src/graph/worker.rs`
intercepts `Edit`/`Write`/`MultiEdit` tool_calls and re-runs the
analyzer on touched files. Stale-edge sweeper runs nightly via
the existing graph worker's cron path. Coalescer extension
dedupes by `(content_hash, analyzer_version)`.

### §6 — Query lane + renderer uplift

New Cypher templates (`code_callers.cypher`, `doc_trail.cypher`,
`blast_radius.cypher`) under the existing
`crates/cortex-workers/cypher/` registry. spec-12 renderer
extended in `cortex-pre-thinking/src/formatter.rs` with three new
sub-blocks. Gold-set extended with 10 questions exercising the
new graph paths; `MRR@10 ≥ 0.75` IT gate continues to apply.

## Impact

- **Affected specs:** `01` (event schema — additive only),
  `07` (graph writer — schema additions + edge taxonomy refresh),
  `11` (query API — new Cypher templates), `12` (pre-thinking — new
  section shapes), `16` (dashboard — graph view colour-coding for
  new labels).
- **Affected code (no new crates):**
  - **New modules:** `crates/cortex-workers/src/graph/analyzer/{mod,rust,typescript,python,go}.rs`,
    `crates/cortex-workers/src/graph/resolver/{mod,module_map,package_map,intra_doc}.rs`,
    `crates/cortex-workers/src/graph/markdown.rs`,
    `crates/cortex-storage/src/external_repos.rs`, 3 new Cypher templates.
  - **Modified:** `crates/cortex-workers/src/graph/{mapper,worker,schema}.rs`
    (analyzer integration + new schema constraints),
    `crates/cortex-cli/src/bin/cortex-bootstrap.rs` (`--graph-static` flag),
    `crates/cortex-pre-thinking/src/formatter.rs` (new sub-blocks),
    `crates/cortex-api/tests/fixtures/relevance-gold.json` (gold-set extension).
  - **Deps added to existing crates:** `pulldown-cmark` (cortex-workers).
- **Breaking:** NO. New edge types and node labels are additive;
  existing 13 edge types and 11 node labels keep their shape.
  Schema migrations land via additive `IF NOT EXISTS` Cypher.
- **Storage delta:** ~75 MB Nexus DB growth on the cortex repo
  (~50 K new edges). Bootstrap cost +4 %. Per-edit graph-write
  +8 ms. Static extraction is $0/month — no Sonnet calls in the
  hot path.
- **User benefit:** Code-only and doc-anchored queries become
  first-class. *"What calls X?"* returns 6-15 callers instead of
  1 self-hit. *"Trace the design behind decision Y"* walks the
  full ADR → spec → file → symbol chain in one bundle. Cross-repo
  questions (`vectorizer-sdk` boundary) become traversable.
  Graph density lifts ~10×; gold-set MRR@10 lifts +0.07 absolute.

## Source

Full design + risk register + phasing rationale:
[`docs/analysis/graph/`](../../../docs/analysis/graph/) — six
files covering current state, gap-by-failure-mode walkthrough,
target schema, extraction pipeline, impact + risks, and the
phased implementation plan that this task tree mirrors.

The analysis was originally written assuming a sibling
`cortex-code-graph` crate. The user's standing rule (no new
crates without explicit approval — every feature folds into the
existing workspace members) reshapes the implementation: every
module proposed in the analysis lives inside `cortex-workers`
or `cortex-storage` instead of a sibling crate. The edge schema,
extraction pipeline, and risk register stay identical.
