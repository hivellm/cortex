# 02 — Gaps, by failure mode

Each gap is named, demonstrated against a concrete query, and pinned
to the missing edge class.

## Gap 1 — No code-to-code edges (the "what calls X?" hole)

**Query:** *"what code calls `rrf_fuse`?"* — a routine
`pre_change_context` ahead of refactoring the fusion algorithm.

**Today:** `rrf_fuse` is a `:Symbol` node with `[DEFINES]` pointing
at `crates/cortex-api/src/fusion.rs`. Zero in-edges. The graph cannot
answer the question. The keyword lane finds **occurrences of the
string `rrf_fuse`** but cannot distinguish a definition from a call
site from a doc reference. The vector lane returns the function
itself as the top hit (high self-similarity) and 4-5 unrelated
ranking utilities at the tail.

**Missing edges:**
- `(:Symbol callee)-[:CALLED_BY]->(:Symbol caller)` — every call
  site emits one edge. `rrf_fuse` has ~6 call sites in
  `crates/cortex-api/src/orchestrator.rs` + service.rs.
- `(:Symbol)-[:USES_TYPE]->(:Symbol)` — every signature mention.
  `FusionConfig` is referenced as a parameter or return type in 11
  signatures across the crate.

**Lift expected:** `pre_change_context` for any function-rename
query goes from "the function definition + its module's docstring"
to "the definition + every call site + every type-use site". Bundle
density per byte ~3×.

## Gap 2 — No file-import structure (the "who depends on this?" hole)

**Query:** *"if I change the public API of `crates/cortex-core/src/events.rs`, what blast radius?"* — pre-change blast assessment.

**Today:** No `IMPORTS_FILE` edge. The agent gets the file as a
snippet, then has to ask the user / the test suite to run the type
check to discover blast radius. Blast estimation costs a full build.

**Missing edges:**
- `(:Artifact dependent)-[:IMPORTS_FILE]->(:Artifact dependency)` — one
  edge per `use cortex_core::events::*` line in any other file.
  Resolving `cortex_core::events` to the actual artifact requires a
  Cargo.toml-mediated package map (`cortex_core` → `crates/cortex-core/`),
  which is cheap to build once at bootstrap.
- `(:Artifact)-[:IMPORTS_EXTERNAL {pkg, version}]->(:ExternalPackage)`
  — for cross-repo / crates.io imports the resolver can't lock to a
  local Artifact. External packages get a `:ExternalPackage` node
  keyed on `name|version`.

**Lift expected:** every `Artifact` in the corpus gains 5-50 dependent
in-edges (the higher end for foundational crates like cortex-core).
"Find all callers of public API X" becomes a single traversal.
Blast-radius estimation via `match (a:Artifact {path: …})<-[:IMPORTS_FILE*1..2]-()` returns the closure in O(degree).

## Gap 3 — No doc→code edges (the "where is this documented?" hole)

**Query:** *"what spec covers the fusion algorithm?"* — `decision_lookup`
ahead of changing `FusionConfig.alpha`.

**Today:** The fusion algorithm is documented in
`docs/specs/11-query-api.md` §Fan-out + fusion. The doc is indexed
as a single `:Artifact` (kind=artifact, family=docs). Zero edge from
that doc to the `rrf_fuse` symbol or the `fusion.rs` artifact. The
keyword lane finds the doc when the user types "fusion algorithm",
but the agent has to *read* the doc to discover which file to read.
A `decision_lookup` for "alpha tuning" returns ADR-002 (sometimes)
and zero documentation context.

**Missing edges:**
- `(:Artifact:doc)-[:MENTIONS]->(:Symbol)` — when the doc references
  a symbol by name in code-fence or backticks. `docs/specs/11.md`
  mentions `rrf_fuse`, `FusionConfig`, `LaneHit` — three edges.
- `(:Artifact:doc)-[:DOCUMENTS]->(:Artifact)` — when the doc
  contains an explicit Markdown link to a code file:
  `[crates/cortex-api/src/fusion.rs](../../crates/cortex-api/src/fusion.rs)`.
  Resolves to one edge per link.
- `(:Artifact:doc)-[:DESCRIBES_PATH]->(:Artifact)` — when the doc
  contains a fenced-code header like
  ```text
  ```rust
  // crates/cortex-api/src/fusion.rs
  ```
  ```
  the `// path/to/file.rs` first-line convention is widely used in
  this repo's docs.
- `(:Symbol)-[:DOCUMENTED_BY]->(:Artifact:doc)` — Rust doc-comments
  carry `[`crate::module::Symbol`]` intra-doc links. Every such
  link inside `///` text is a backwards edge from a code Symbol to
  the documenting doc artifact.

**Lift expected:** `decision_lookup` for any documented topic surfaces
the spec, the file, AND the symbols all in one bundle. Today's bundle
shows the file + a fragment of the spec text; the new bundle gives
the full chain spec→file→symbols→callers in 2 hops.

## Gap 4 — No spec↔spec edges (the "trace the design back" hole)

**Query:** *"why did we pick the recency-decay table in §3.1?"* —
`decision_lookup` against the phase11i decision.

**Today:** The recency-decay table is in
`docs/specs/11-query-api.md` §Fusion algorithm + the operator
handbook `docs/cortex/relevance-tuning.md`. Both docs cite each
other by name in markdown text but no edges. The phase11i
implementation task references the analysis under
`docs/analysis/organize/04-relevance-axes.md` which itself cites
the analysis under `docs/analysis/relevance/*`. Every link is a
literal markdown `[text](path)`. Zero traversable edges.

**Missing edges:**
- `(:Artifact:doc)-[:LINKS_TO]->(:Artifact:doc)` — every markdown
  link `[label](path/to/other.md)` produces one edge. Path is
  resolved relative to the source file's directory.
- `(:Decision)-[:CITES]->(:Artifact:doc|:Decision|:Analysis)` —
  ADRs (`.rulebook/decisions/*.md`) carry citations both in
  free-form text and in the `links: Vec<String>` payload field.
  Currently the payload field is stored but never resolved; the
  free-form text is never parsed.
- `(:Spec spec)-[:REFERENCES]->(:Spec other)` — section headers
  like "see spec 12" or `docs/specs/12-pre-thinking-injection.md`
  references inside spec body text.

**Lift expected:** `decision_lookup` walks the citation chain.
"Why does relevance-tuning.md set λ=0.02?" returns the operator
handbook + spec 11 §3.1 + the phase11i analysis + the ADR — all in
one bundle, deduped, ranked by hop distance.

## Gap 5 — No code→doc backlink (the "is this documented?" hole)

**Query:** *"does this function have a spec?"* — pre-change context
for a refactor.

**Today:** Same as Gap 3 in reverse. A symbol's docstring may
reference a spec via Rust intra-doc syntax `[`crate::path::Symbol`]`
or by markdown-style link, but no graph emitter walks the
docstring AST.

**Missing edges:**
- `(:Symbol)-[:DOCUMENTED_BY]->(:Artifact:doc)` — every Rust
  doc-comment intra-doc link, every TS / Py docstring URL pattern.
  The Tree-sitter pass that already extracts the symbol can also
  pick up the leading comment block.
- `(:Symbol)-[:DOCSTRING_REFERENCES]->(:Symbol)` — intra-doc
  `[`other_symbol`]` references between Rust symbols. Common
  pattern in cortex-core / cortex-api.

**Lift expected:** `pre_change_context` on a refactor surfaces the
symbol's doc, plus every spec section the docstring referenced —
without the agent having to grep for the symbol name in the docs
folder.

## Gap 6 — No cross-repo symbol resolution (the "external API" hole)

**Query:** *"what version of the Vectorizer SDK does cortex-api use,
and what changed between releases?"* — `similar_problems`.

**Today:** `cortex-api`'s `Cargo.toml` declares `vectorizer-sdk = "3.2.0"`.
The `use vectorizer_sdk::HnswSearch` line in `vectorizer_lane.rs`
gets parsed by the chunker for symbol extraction (`HnswSearch` does
NOT appear in the symbol set because it is not declared in this file
— the chunker correctly skips it). Zero edge.

**Missing edges:**
- `(:Artifact)-[:IMPORTS_EXTERNAL {pkg, version}]->(:ExternalPackage)`
- `(:ExternalPackage)-[:HOSTED_AT]->(:Repo external)` for HiveLLM-
  internal SDKs (Vectorizer / Nexus / Synap / Lexum) where the
  external repo IS in `e:/HiveLLM/`. The repo discovery pass at
  bootstrap sees the path and can map `vectorizer-sdk` →
  `e:/HiveLLM/Vectorizer/`.
- `(:ExternalPackage)-[:VERSION]->(:Version)` — surface versioning
  so a `decision_lookup` for "why did we bump from 3.0.3 to 3.2.0?"
  follows the Cargo.toml diff to the corresponding ADR.

**Lift expected:** Cross-repo questions become first-class. The
agent reasons over the SDK boundary instead of treating it as a
black box.

## Gap 7 — No knowledge / learning / consolidation citations

**Query:** *"what did we learn about RRF tuning?"* — `similar_problems`.

**Today:** `.rulebook/learnings/*.md` produces `:Memory` nodes
keyed on `event_id` (phase10e). Their content cites code paths,
spec sections, and other learnings in markdown. None of those
become edges.

**Missing edges:**
- `(:Learning)-[:CITES]->(:Artifact|:Symbol|:Decision|:Spec)` — same
  Markdown extraction pipeline as Gap 4 + Gap 5, applied to the
  knowledge / learning / consolidation payload bodies.
- `(:Consolidation)-[:DERIVED_FROM]->(:Turn|:ToolCall|:Decision)` —
  the phase11j payload carries `source_event_ids[]` already; the
  graph layer can materialise these as explicit edges so a query
  walks "consolidated takeaway → original turn" in one hop.

**Lift expected:** the `Past sessions` overlay's "see related
sessions" pivot lands real citations instead of vector-similarity
guesses. Consolidations stop being floating summaries and become
the navigable hub of the curated layer.

## Failure-mode summary

| Gap | Missing edge class       | Worst-hit query intent     | Today's bundle | New bundle |
| --- | ------------------------ | -------------------------- | -------------- | ---------- |
| 1   | code↔code (intra)        | `pre_change_context` rename| 1 file         | file + 6-15 callers |
| 2   | file imports             | blast-radius                | 0 dependents   | 5-50 dependents |
| 3   | doc→code                 | `decision_lookup`           | doc OR code    | doc + code |
| 4   | spec↔spec                | trace-the-design            | 1 doc          | citation chain |
| 5   | code→doc                 | "is this documented?"       | 0 hits         | symbol + spec section |
| 6   | cross-repo               | "external API" questions    | nothing        | SDK boundary visible |
| 7   | knowledge / consolidation citations | `similar_problems` | takeaway only  | takeaway + sources |

Each gap has a deterministic, static extraction path — Tree-sitter
+ Markdown parser + path/symbol resolver. The next file
([03-target-graph.md](./03-target-graph.md)) defines the schema for
all of it.
