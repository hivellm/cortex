## 1. Tree-sitter analyzer + Rust support (in cortex-workers)

- [ ] 1.1 New module `crates/cortex-workers/src/graph/analyzer/mod.rs` — `CodeAnalyzer` trait + `CodeEdge` / `ResolutionTarget` / `EdgeType` types; per-language analyzer signature; pulldown-cmark + tree-sitter-rust workspace deps appended to `cortex-workers/Cargo.toml`
- [ ] 1.2 `crates/cortex-workers/src/graph/analyzer/rust.rs` — Rust analyzer with four Tree-sitter queries (use_decl, call_expr, type_use, impl_block) producing `Vec<CodeEdge>` per file; reuses the existing grammar registry from `embedder/chunker_code.rs`
- [ ] 1.3 `crates/cortex-workers/src/graph/resolver/` (new submodule) — `ModuleMap` builder (Cargo.toml + `mod foo;` walk), three-tier `SymbolResolver` (local-file / intra-crate / external), `PackageMap` reading workspace `[dependencies]`, new `crates/cortex-storage/src/external_repos.rs` TOML loader for HiveLLM-internal SDK declarations
- [ ] 1.4 GraphPatch builder bridging `CodeEdge` → `EdgeOp` consumed by `cortex-workers/src/graph/mapper.rs`; new schema constraints (`spec_path`, `external_package_natural_key`, `doc_section_natural_key`) appended to `schema::SCHEMA_STATEMENTS`
- [ ] 1.5 12 unit tests in `analyzer/rust.rs::tests` pinning every query against fixture Rust files (use_decl / call_expr / type_use / impl_block coverage)
- [ ] 1.6 Resolver IT `crates/cortex-workers/tests/graph_resolver_it.rs` — 8 cases pinning tier-1/tier-2/tier-3 dispatch + the `:UNRESOLVED_IMPORT` fallback
- [ ] 1.7 End-to-end IT `crates/cortex-workers/tests/graph_analyzer_rust_it.rs` — synthesise a 3-file Rust crate, run the analyzer, assert the edge set matches the proposal verbatim

## 2. Multi-language analyzers (TS / Python / Go)

- [ ] 2.1 `crates/cortex-workers/src/graph/analyzer/typescript.rs` — TS + TSX analyzer covering import_statement, call_expression, class extends, member call resolution
- [ ] 2.2 `crates/cortex-workers/src/graph/analyzer/python.rs` — import / call / class inheritance queries
- [ ] 2.3 `crates/cortex-workers/src/graph/analyzer/go.rs` — import / call / type-decl queries
- [ ] 2.4 Per-language unit tests inline in each analyzer module covering import / call / type-use extraction (same shape as §1.5)
- [ ] 2.5 Multi-language IT `crates/cortex-workers/tests/graph_analyzer_multi_lang_it.rs` — workspace mixing Rust + TS + Python emits coherent cross-language edges (Rust crate's `vectorizer-sdk` resolves the same way a TS workspace import does)

## 3. Markdown analyzer + doc↔code edges (in cortex-workers)

- [ ] 3.1 `crates/cortex-workers/src/graph/markdown/mod.rs` — `MarkdownAnalyzer` entry point, `MarkdownEdge` type, pulldown-cmark walker scaffold
- [ ] 3.2 `markdown/links.rs` — emits `:LINKS_TO`, `:DOCUMENTS`, `:LINKS_TO_SECTION`; resolves relative paths against the workspace root
- [ ] 3.3 `markdown/sections.rs` — emits `:DocSection` (GitHub-flavoured slug) + implicit `:CONTAINS` parent edges
- [ ] 3.4 `markdown/mentions.rs` — backtick-token symbol mentions with three-tier disambiguation; `confidence` prop on every edge
- [ ] 3.5 `markdown/code_blocks.rs` — fenced-code first-line `// path/to/file.rs` extraction → `:DESCRIBES_PATH`
- [ ] 3.6 `crates/cortex-workers/src/graph/resolver/intra_doc.rs` — Rust `///` intra-doc parser via Tree-sitter, emits `:DOCUMENTED_BY` + `:DOCSTRING_REFERENCES`
- [ ] 3.7 Markdown analyzer IT `crates/cortex-workers/tests/graph_markdown_it.rs` — 10 cases covering link / section / mention / code-block / intra-doc extraction
- [ ] 3.8 Mention-precision IT `crates/cortex-workers/tests/graph_mentions_precision_it.rs` — 50 hand-curated mentions, ≥ 95 % precision against the resolved-symbol assertion set

## 4. Decision / Knowledge / Learning / Consolidation citations

- [ ] 4.1 Extend the markdown analyzer to walk `Decision`/`Knowledge`/`Learning`/`Consolidation` payload bodies; each becomes an in-bound source for `:CITES` edges
- [ ] 4.2 Resolve `DecisionPayload.links[]` (currently dead in the graph layer) into typed `:CITES` edges against the right Artifact / Decision / Analysis target via the same path resolver
- [ ] 4.3 Materialise `:DERIVED_FROM` edges from `Consolidation.payload.source_event_ids[]` so the curated layer is navigable; wires into `cortex-workers/src/graph/mapper.rs::emit_memory` (the path Consolidation envelopes already ride)
- [ ] 4.4 Citation-chain IT `crates/cortex-workers/tests/graph_citation_chain_it.rs` — walks an ADR → Spec → Analysis → Code chain and asserts the full chain is traversable in 4 hops with `confidence ≥ 0.9`

## 5. Bootstrap + live triggers

- [ ] 5.1 `cortex-bootstrap --graph-static` flag added to `crates/cortex-cli/src/bin/cortex-bootstrap.rs` — walks the workspace once, runs code + markdown analyzers, emits envelopes through the existing archive sink (phase11i §1.7) so `archive_loader` re-reads on graph-worker boot
- [ ] 5.2 Wire the live trigger into `crates/cortex-workers/src/graph/worker.rs` — when a `Kind::Artifact` event lands with content_hash that differs from the previous one, run the analyzer and merge the resulting patch with the structural patch
- [ ] 5.3 Stale-edge sweeper — extend the graph worker's existing nightly cron to walk edges whose `source_event_id` references an event whose content_hash is no longer current; emits delete patches via the graph writer's `delete_edges_by_filter` surface (lands here if not pre-existing)
- [ ] 5.4 Coalescer extension in `crates/cortex-workers/src/graph/coalescer.rs` — per-session dedupe on `(content_hash, analyzer_version)` so a search-and-replace burst doesn't re-emit identical patches
- [ ] 5.5 End-to-end IT `crates/cortex-workers/tests/graph_bootstrap_then_live_it.rs` — bootstrap a fixture workspace, simulate a Live edit, assert the graph reflects both passes correctly + idempotent re-emit

## 6. Query lane + renderer uplift

- [ ] 6.1 New Cypher templates in `crates/cortex-workers/cypher/`: `code_callers.cypher` (`:CALLS` 1-hop), `doc_trail.cypher` (`:CITES` chain), `blast_radius.cypher` (`:IMPORTS_FILE*1..2`); each ships with a unit test asserting the rendered Cypher
- [ ] 6.2 Extend `crates/cortex-pre-thinking/src/formatter.rs` — surface graph-traversal hits in the `Past sessions` / `Consolidated context` sections under three new sub-blocks: `Connected files (via IMPORTS_FILE)`, `Documented under (via DOCUMENTED_BY)`, `Cited from (via CITES)`
- [ ] 6.3 Update `docs/specs/12-pre-thinking-injection.md` §Output with the new section shapes + worked example showing the spec → file → symbols → callers chain
- [ ] 6.4 Extend `crates/cortex-api/tests/fixtures/relevance-gold.json` with 10 new questions exercising the new graph paths (intent split: 4 pre_change_context, 3 decision_lookup, 2 similar_problems, 1 free_search); same `MRR@10 ≥ 0.75` IT gate applies via `phase11i §4.5` IT
- [ ] 6.5 Operator handbook `docs/cortex/graph-tuning.md` — how to spot a missing edge, how to inspect resolver tier mismatches, how to flag a false-positive `:MENTIONS`, how to register a new HiveLLM-internal SDK in `external_repos.toml`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 7.1 Update or create documentation covering the implementation — CHANGELOG entry for phase11k; `docs/architecture.md` §6 (graph correlation layer alongside retrieval lanes); `docs/specs/07-graph-writer.md` §Schema + §Edge types refresh; `docs/cortex/graph-tuning.md` (already produced by §6.5)
- [ ] 7.2 Write tests covering the new behavior — every IT named in §1-§6 lands; coverage ≥ 95 % on the new `cortex-workers/src/graph/analyzer/`, `resolver/`, `markdown/` modules; the citation-chain IT is the headline acceptance gate
- [ ] 7.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`, full IT suite gated by `CORTEX_*_IT=1` (CODE_GRAPH, MARKDOWN_GRAPH, CITATION_CHAIN); all green
- [ ] 7.4 Capture learnings: `rulebook_learn_capture` for any non-obvious finding from the implementation (Tree-sitter query gotchas, resolver disambiguation tuning, prompt → static-extraction trade-offs)
- [ ] 7.5 Capture decision: `rulebook_decision_create` for the static-extraction-over-LSP choice (§2.3 risk + §1 design rationale) pinned against a quantitative reassessment trigger (when does the syntactic resolver's ~10 % wrong-target rate become unacceptable?)
