# 06 — Implementation plan

Maps onto a Rulebook task tree at
`.rulebook/tasks/phase11k_graph_correlations/`. Phasing prioritises
the highest-relevance-lift edges first so each commit ships a
visible operator win.

## Phase 1 — Tree-sitter analyzer + Rust support (in cortex-workers)

Lands the infrastructure + the first language. Validates the
extraction → resolution → emission pipeline end-to-end before
expanding to other languages or markdown. **No new crate** — the
analyzer modules sit inside `cortex-workers/src/graph/` alongside
the existing mapper / worker, per
[04-extraction-pipeline.md §1](./04-extraction-pipeline.md#1-module-placement-no-new-crate).

- **§1.1** New module tree under `crates/cortex-workers/src/graph/analyzer/`
  with the layout from §1 of the extraction pipeline doc.
  `cortex-workers/Cargo.toml` gains `pulldown-cmark` (the only
  new dep — Tree-sitter grammars are already present from the
  embedder's chunker_code).
- **§1.2** `CodeAnalyzer` trait + `CodeEdge` / `ResolutionTarget`
  types in `cortex-workers/src/graph/analyzer/mod.rs`.
- **§1.3** Rust analyzer (`graph/analyzer/rust.rs`) with the four
  queries from [04-extraction-pipeline.md §2.1](./04-extraction-pipeline.md#21-rust):
  use_decl, call_expr, type_use, impl_block. Output a
  `Vec<CodeEdge>` per file.
- **§1.4** Resolver skeleton (`graph/resolver/`): `ModuleMap`
  builder reading `Cargo.toml` + walking `mod foo;` declarations,
  three-tier `SymbolResolver`, `PackageMap` reading workspace
  `[dependencies]`. New `cortex-storage/src/external_repos.rs`
  TOML loader for cross-repo SDK paths.
- **§1.5** GraphPatch builder bridging `CodeEdge` → `EdgeOp`
  inside `cortex-workers/src/graph/mapper.rs`. New schema
  constraints (`spec_path`, `external_package_natural_key`,
  `doc_section_natural_key`) appended to `schema::SCHEMA_STATEMENTS`.
- **§1.6** Tree-sitter grammar coverage: 12 unit tests inline in
  `analyzer/rust.rs::tests` pinning every query against fixture
  Rust files.
- **§1.7** Resolver IT
  (`crates/cortex-workers/tests/graph_resolver_it.rs`): 8 cases
  pinning tier-1/tier-2/tier-3 dispatch + the unresolved-import
  fallback.
- **§1.8** End-to-end IT
  (`crates/cortex-workers/tests/graph_analyzer_rust_it.rs`): seed
  a 3-file synthetic Rust crate, run the analyzer, assert the
  edge set matches the proposal verbatim.

## Phase 2 — TypeScript + Python + Go analyzers

- **§2.1** TS analyzer (`src/code/typescript.rs`) — covers .ts +
  .tsx via the existing `tree-sitter-typescript` workspace dep.
- **§2.2** Python analyzer (`src/code/python.rs`).
- **§2.3** Go analyzer (`src/code/go.rs`).
- **§2.4** Per-language IT covering import / call / type-use
  extraction. Same shape as §1.6.
- **§2.5** Multi-language fixture IT (`tests/multi_lang_it.rs`):
  ensures one workspace mixing Rust + TS + Python emits coherent
  cross-language edges (a Rust crate's `vectorizer-sdk` import
  resolves the same way a TS import does).

## Phase 3 — Markdown analyzer + doc↔code edges

- **§3.1** `MarkdownAnalyzer` (`src/markdown/mod.rs`) with
  pulldown-cmark walker scaffold + `MarkdownEdge` type.
- **§3.2** Link extractor (`src/markdown/links.rs`) — emits
  `:LINKS_TO`, `:DOCUMENTS`, `:LINKS_TO_SECTION`.
- **§3.3** Section extractor (`src/markdown/sections.rs`) —
  emits `:DocSection` + `:CONTAINS` parent edges.
- **§3.4** Symbol mention extractor (`src/markdown/mentions.rs`)
  with three-tier disambiguation. `confidence` prop drives the
  renderer's filter.
- **§3.5** Fenced-code path-header extractor
  (`src/markdown/code_blocks.rs`) — emits `:DESCRIBES_PATH`.
- **§3.6** Rust intra-doc parser (`src/resolver/intra_doc.rs`) —
  reads `///` doc-comments via Tree-sitter, extracts intra-doc
  `[`crate::path::Sym`]` references, emits
  `:DOCUMENTED_BY` + `:DOCSTRING_REFERENCES`.
- **§3.7** Markdown analyzer IT (`tests/markdown_it.rs`): 10 cases
  covering link / section / mention / code-block / intra-doc
  extraction.
- **§3.8** Mention-precision IT (`tests/mentions_precision_it.rs`)
  — 50 hand-curated mentions, asserts ≥ 95 % precision against
  the resolved-symbol assertion set.

## Phase 4 — Decision / Knowledge / Learning / Consolidation citations

- **§4.1** Extend the markdown analyzer to walk
  `Decision`/`Knowledge`/`Learning`/`Consolidation` payload
  bodies. Each becomes an in-bound source for `:CITES` edges.
- **§4.2** Resolve `Decision.payload.links[]` (an existing field
  that's currently unused at the graph layer) into typed edges.
- **§4.3** Materialise `:DERIVED_FROM` edges from
  `Consolidation.payload.source_event_ids[]`. Phase11j already
  carries the field; phase11k just wires it to graph patches.
- **§4.4** ADR / Spec cross-reference IT
  (`tests/citation_chain_it.rs`) walks the
  ADR → Spec → Analysis → Code chain and asserts the full
  citation chain is traversable in 4 hops.

## Phase 5 — Bootstrap + live triggers

- **§5.1** Add `--graph-static` flag to `cortex-bootstrap` that
  walks the workspace once and emits the static-edge envelopes
  through the existing archive sink (phase11i §1.7).
- **§5.2** Wire the live trigger into `cortex-workers/src/graph/worker.rs`
  — when a `Kind::Artifact` event lands with content_hash that
  differs from the previous one, run the analyzer and merge the
  resulting patch with the structural patch.
- **§5.3** Stale-edge sweeper: nightly cron walks edges whose
  `source_event_id` references an event whose content_hash is no
  longer the current one for the artifact. Sweep emits a delete
  patch via the existing graph writer's `delete_edges_by_filter`
  surface (lands in this task if not already present).
- **§5.4** Coalescer extension: per-session dedupe on
  `(content_hash, analyzer_version)`.
- **§5.5** End-to-end IT
  (`tests/end_to_end_bootstrap_then_live_it.rs`): bootstrap a
  fixture workspace, then simulate a Live edit, assert the graph
  reflects both passes correctly + idempotent re-emit.

## Phase 6 — Query lane + renderer uplift

- **§6.1** New Cypher templates in
  `crates/cortex-workers/cypher/`:
  - `code_callers.cypher` — `:CALLS` traversal for "what calls X?"
  - `doc_trail.cypher` — `:CITES` chain for `decision_lookup`
  - `blast_radius.cypher` — `:IMPORTS_FILE*1..2` for blast assessment
- **§6.2** Extend the spec-12 renderer's `Past sessions` /
  `Consolidated context` sections to also surface the new graph
  hits (`Connected files (via IMPORTS_FILE)`,
  `Documented under (via DOCUMENTED_BY)`, `Cited from (via CITES)`).
- **§6.3** Update `docs/specs/12-pre-thinking-injection.md` with
  the new section shapes + worked example.
- **§6.4** Extend the gold-set fixture
  (phase11i §4.4) with 10 questions that exercise the new graph
  paths. Same `MRR@10 ≥ 0.75` IT gate applies.
- **§6.5** Operator handbook update under
  `docs/cortex/graph-tuning.md`: how to spot a missing edge,
  how to inspect resolver tier mismatches, how to flag a
  false-positive `:MENTIONS`.

## Phase 7 — Tail (mandatory)

- **§7.1** Update / create documentation covering the
  implementation — `CHANGELOG` entry for phase11k;
  `docs/architecture.md` §6 (graph correlation layer alongside
  retrieval lanes); `docs/specs/07-graph-writer.md` §Schema +
  §Edge types refresh; `docs/cortex/graph-tuning.md` (already
  produced by §6.5).
- **§7.2** Write tests covering the new behavior — every IT named
  in §1-§6 lands; coverage ≥ 95 % on `crates/cortex-code-graph/`;
  the citation-chain IT is the headline acceptance gate.
- **§7.3** Run tests and confirm they pass — `cargo check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
  `cargo test --all-features`, IT suites gated by
  `CORTEX_*_IT=1` (CODE_GRAPH, MARKDOWN_GRAPH, CITATION_CHAIN).
- **§7.4** Capture learnings via `rulebook_learn_capture` —
  Tree-sitter query gotchas, resolver disambiguation tuning,
  prompt → static-extraction trade-offs.
- **§7.5** Capture decision via `rulebook_decision_create` — the
  static-extraction-over-LSP choice (§2.3 risk + §1 design)
  pinned against a quantitative reassessment trigger.

## Sequencing rationale

Phases 1-2 land the analyzer infrastructure + cover ~80 % of the
corpus by language. Phase 3 layers in the doc↔code edges that are
the highest-impact win for `decision_lookup` and `pre_change_context`.
Phase 4 connects ADRs / consolidations into the citation graph.
Phase 5 wires it all to the live worker pipeline. Phase 6 makes the
new edges visible to the agent through the renderer + Cypher
templates. Phase 7 closes the loop with docs + tests.

Each phase is a multi-commit chunk with its own ITs gating
the merge. The gold-set IT in §6.4 is the headline acceptance gate
for the whole phase tree.

## Estimated scope

| Phase | Sub-items | Days est. |
| ----- | --------- | --------- |
| 1     | 8         | 4         |
| 2     | 5         | 3         |
| 3     | 8         | 4         |
| 4     | 4         | 2         |
| 5     | 5         | 3         |
| 6     | 5         | 2         |
| 7     | 5         | 1         |
| **Total** | **40**| **~19 days** |

Comparable to phase11i (37 items / ~3 weeks). Phase11k can run
partly parallel to phase11j's remaining work (§4 query lane, §5
pruner, §6 tail) since the two phases share no common files.
