# 03 — Target graph schema

The schema below extends the existing structural layer
([01-current-state.md §3](./01-current-state.md#3-edge-types-emitted-today))
with three new edge classes (A, B, C) plus four new node labels.

## 1. New node labels

| Label                                      | Natural key            | Source                                                 |
| ------------------------------------------ | ---------------------- | ------------------------------------------------------ |
| `:Spec`                                    | `path`                 | `docs/specs/*.md` (specialised `:Artifact:doc`)        |
| `:ExternalPackage`                         | `name|version`         | Cargo.toml / package.json / pyproject dependency block |
| `:Concept` (already implicit)              | `entity_type|identifier` | Sonnet semantic layer                                |
| `:DocSection`                              | `path|heading_anchor`  | Markdown H1-H4 sections inside a `:Spec` or `:Artifact:doc` |

`:Spec` is technically `:Artifact` + a discriminator, but giving it
its own label means the dashboard can fan out spec→spec traversals
without scanning every Artifact. `:DocSection` lets a `:CITES` edge
target a specific `## §3.4` instead of the whole document — important
because long specs (12-pre-thinking, 11-query-api) are 800-1200 lines
and a section-level edge is what an ADR actually wants.

The Knowledge / Learning / Consolidation events already get
distinguishable behaviour via `entity_type` props; phase11j §3 added
their canonical labels to the routing layer. They become first-class
edge sinks below without further node-shape work.

## 2. Edge class A — code structure

Static extraction via Tree-sitter queries. One pass per source file
during bootstrap and on every Edit/Write tool_call.

| Edge                                                  | Trigger                                          |
| ----------------------------------------------------- | ------------------------------------------------ |
| `(:Artifact a)-[:IMPORTS_FILE]->(:Artifact b)`        | `use mod::path::*` resolved to a local file      |
| `(:Artifact)-[:IMPORTS_EXTERNAL {pkg, version}]->(:ExternalPackage)` | `use external_crate::…` not in workspace |
| `(:Symbol caller)-[:CALLS]->(:Symbol callee)`         | every call expression in the function body       |
| `(:Symbol)-[:USES_TYPE]->(:Symbol)`                   | type references in signatures + struct fields    |
| `(:Symbol impl)-[:IMPLEMENTS]->(:Symbol trait)`       | Rust `impl Trait for Type` blocks                |
| `(:Symbol child)-[:EXTENDS]->(:Symbol parent)`        | TS / Py / Java class inheritance                 |
| `(:Symbol)-[:RE_EXPORTS]->(:Symbol)`                  | Rust `pub use` re-exports                        |

### Resolver contract

A `SymbolResolver` resolves `(repo, source_module_path, raw_target)`
into a `:Symbol` natural key. Three lookup tiers:

1. **Local-file lookup** — the target lives in the same file.
   Cheapest. ~80 % of intra-function calls.
2. **Intra-crate lookup** — walk the module tree from the file's
   crate root using a pre-built `(crate_root, module_path) →
   artifact_path` map. ~15 %.
3. **Cross-crate / external** — match against the workspace's
   declared dependencies (Cargo.toml `[dependencies]`). When the
   declared dep maps to another workspace member, the edge becomes
   intra-workspace (`:IMPORTS_FILE` cross-repo). Otherwise emit
   `:IMPORTS_EXTERNAL`.

Edges that fail every tier go to `(:Artifact)-[:UNRESOLVED_IMPORT
{raw}]->(:Concept)` so the graph viewer surfaces them for triage
without polluting the resolved-edge counts.

### Resolution stability

Re-runs MUST be idempotent. The natural-key shape
(`repo|lang|qualified_name` for Symbol; `repo|path|content_hash` for
Artifact) means a re-emit produces the same edge identity unless
content changes. Edge props carry the `source_event_id` of the
extracting tool_call so the dashboard can audit which event
introduced an edge.

## 3. Edge class B — doc↔code

Static extraction via a Markdown link parser (`pulldown-cmark` or
`comrak`) + a regex pass for backtick-fenced symbol mentions and
fenced-code-block path headers.

| Edge                                                  | Trigger                                          |
| ----------------------------------------------------- | ------------------------------------------------ |
| `(:Artifact:doc)-[:DOCUMENTS]->(:Artifact)`           | Markdown link `[…](path/to/file.{rs,ts,…})`      |
| `(:Artifact:doc)-[:MENTIONS]->(:Symbol)`              | Backtick-fenced ``ident`` matching a known Symbol natural_key |
| `(:Artifact:doc)-[:DESCRIBES_PATH]->(:Artifact)`      | Fenced-code first-line `// path/to/file.rs`      |
| `(:Symbol)-[:DOCUMENTED_BY]->(:Artifact:doc)`         | Rust `///` intra-doc `[`crate::Sym`]` references |
| `(:Symbol)-[:DOCSTRING_REFERENCES]->(:Symbol)`        | Intra-doc cross-symbol references                |

### Disambiguation rules

The `:MENTIONS` edge is the noisiest — backtick-fenced text in docs
matches lots of false positives. Three rules to keep precision high:

1. **Symbol resolver tier** — the backtick-fenced token must match
   an existing `:Symbol.qualified_name` OR `:Symbol.name` in the
   same repo. Tokens that match nothing are dropped.
2. **Length floor** — single-character or two-character tokens are
   skipped (avoid `let x = …` matches). Minimum length 3.
3. **Context filter** — when the doc is under `docs/specs/` or
   `.rulebook/decisions/`, allow ambiguous matches (intent is
   high-recall design discussion). When the doc is a generic
   README, require the exact `crate::module::name` form.

False-positive rate target: ≤ 5 % via spot-check against a curated
50-mention sample.

## 4. Edge class C — cross-doc + provenance

| Edge                                                  | Trigger                                          |
| ----------------------------------------------------- | ------------------------------------------------ |
| `(:Artifact:doc)-[:LINKS_TO]->(:Artifact:doc)`        | Markdown link `[…](other.md)`                    |
| `(:Artifact:doc)-[:LINKS_TO_SECTION]->(:DocSection)`  | Markdown link with `#anchor`                     |
| `(:Decision)-[:CITES]->(:Artifact|:Decision|:Analysis|:Knowledge|:Learning)` | ADR body links + payload `links[]` |
| `(:Spec)-[:REFERENCES]->(:Spec)`                      | Spec body cross-references                       |
| `(:Analysis)-[:CITES]->(:Artifact|:Decision|:Spec)`   | Analysis body links                              |
| `(:Knowledge|:Learning|:Consolidation)-[:CITES]->(...)` | Same Markdown extraction over payload body     |
| `(:Consolidation)-[:DERIVED_FROM]->(:Turn|:ToolCall|:Decision)` | Materialised from `payload.source_event_ids[]` |

### Section-level granularity

Markdown links carrying `#anchor` (`docs/specs/12.md#output`)
resolve to a `:DocSection` node. The extractor walks each markdown
file once at bootstrap, emits one `:DocSection` per `#`/`##`/`###`
heading with the GitHub-flavoured slug as the anchor. A `:Spec` or
`:Artifact:doc` is the parent (`:CONTAINS`). Section-level edges
reduce false-positive `:LINKS_TO` traversals — a citation of "spec
12 §Output" lands on the section, not the whole 1200-line doc.

## 5. Schema bootstrap additions

Constraints (extend `schema.rs::SCHEMA_STATEMENTS`):

```cypher
CREATE CONSTRAINT spec_path IF NOT EXISTS
  FOR (s:Spec) REQUIRE s.path IS UNIQUE;

CREATE CONSTRAINT external_package_natural_key IF NOT EXISTS
  FOR (p:ExternalPackage) REQUIRE p.natural_key IS UNIQUE;

CREATE CONSTRAINT doc_section_natural_key IF NOT EXISTS
  FOR (s:DocSection) REQUIRE s.natural_key IS UNIQUE;
```

Indexes:

```cypher
CREATE INDEX symbol_qualified_name IF NOT EXISTS
  FOR (s:Symbol) ON (s.repo, s.qualified_name);
-- Speeds up the doc-MENTIONS resolver's name lookup.

CREATE INDEX artifact_path IF NOT EXISTS
  FOR (a:Artifact) ON (a.path);
-- Speeds up the IMPORTS_FILE resolver's path-only lookup
-- (when content_hash isn't pinned).

CREATE INDEX doc_section_doc IF NOT EXISTS
  FOR (s:DocSection) ON (s.doc_path);
-- Speeds up "all sections of doc X" traversal.
```

## 6. Edge-prop conventions

Every new edge carries a uniform prop bag:

| Prop                | Type                | Meaning                              |
| ------------------- | ------------------- | ------------------------------------ |
| `source_event_id`   | string              | The event that emitted this edge     |
| `source_path`       | string              | Repo-relative source-file path       |
| `source_line`       | int (when available)| Line number for code edges           |
| `kind`              | string              | Sub-discriminator (e.g. `function_call`, `type_use`, `markdown_link`) |
| `confidence`        | float (0.0-1.0)     | Resolver confidence (1.0 for direct path resolution; <1.0 for symbol-name resolution) |
| `extracted_by`      | string              | `tree-sitter-rust@0.21` / `pulldown-cmark@0.10` / `static-resolver@v1` |

## 7. Backwards compatibility

Every existing edge keeps its label and prop bag. The new edges sit
alongside without renaming or restructuring. The Sonnet semantic
layer continues to fire (it catches edges the static layer misses,
like "this turn DISCUSSES decision DEC-0042" which has no
extractable Markdown link). Static and semantic edges may overlap
on `:DEFINES` / `:REFERENCES` / `:IMPLEMENTS` — the dashboard
de-dupes by `(from_key, edge_type, to_key)` triple.

## 8. What's deliberately NOT in the target

- **Type inference / borrow checking** — call resolution is
  best-effort syntactic. We resolve `foo.bar()` to `(:Symbol bar)`
  by name+arity, not by type. Wrong on overloaded methods. We
  accept the imprecision (~10 % wrong-target rate) because the
  bundle still surfaces the right file, and a perfect resolver
  needs the full Rust type-checker.
- **Macro expansion** — Rust macros that generate calls are
  invisible to syntactic analysis. The graph emits no edge for
  generated code. Same trade-off as IDE go-to-definition.
- **Build-time / test-only edges** — `dev-dependencies` and
  `[build-dependencies]` produce edges but with `kind = "dev"` /
  `kind = "build"` so traversal queries can filter them out for
  prod-relevant questions.

## 9. End-to-end picture

A 20-turn coding session that touches `crates/cortex-api/src/fusion.rs`
under the new schema:

```text
Session ─[HAS_TURN]─→ Turn ─[HAS_TOOL_CALL]─→ ToolCall ─[TOUCHED]─→ Artifact:fusion.rs
                                                                        │
                                                                  [DOCUMENTED_BY]
                                                                        ↓
                                                            Artifact:doc:11-query-api.md
                                                                        │
                                                                  [LINKS_TO]
                                                                        ↓
                                                            Artifact:doc:relevance-tuning.md
                                                                        │
                                                                  [CITED_BY ←]
                                                                        ↑
                                                                Decision:DEC-0042
                                                                        ↑
                                                                  [DERIVED_FROM ←]
                                                                        ↑
                                                            Consolidation (decision_trace)

Artifact:fusion.rs ─[IMPORTS_FILE]─→ Artifact:lanes.rs
                  ─[IMPORTS_EXTERNAL]─→ ExternalPackage:chrono@0.4
                  ─[DEFINES ←]─ Symbol:rrf_fuse
                                        │
                                  [CALLED_BY ←]
                                        ↑
                                  Symbol:run (in orchestrator.rs)
                                        ↑
                                  [CALLS]
                                        │
                                  Symbol:handle (in service.rs)
```

The bundle returned for *"what calls `rrf_fuse`?"* (Gap 1) walks
`(:Symbol {name: "rrf_fuse"})<-[:CALLS]-(:Symbol)` once and gets the
6 caller symbols + their parent files in a single hop.

The bundle for *"trace the design behind alpha = 0.7"* (Gap 4)
walks ADR `links[]` → `:Spec` → §section → cited learning →
underlying turn — fully traversable.
