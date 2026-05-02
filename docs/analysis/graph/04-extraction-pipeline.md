# 04 — Extraction pipeline

This file specifies *how* every edge from
[03-target-graph.md](./03-target-graph.md) is extracted, where the
code lives, and how idempotency / cost stay bounded.

## 1. Module placement (no new crate)

Per the project's standing rule (no new crates without explicit
approval — every feature folds into an existing workspace member),
the analyzers live inside **`cortex-workers`**, alongside the
existing graph mapper + worker. The `embedder/` subtree already
hosts the Tree-sitter grammar registry the analyzers reuse, and
the `graph/` subtree owns the patch surface they emit into.

Module layout (additions only — every existing file is unchanged):

```
crates/cortex-workers/src/graph/
├── analyzer/                  # NEW — static code analyzers
│   ├── mod.rs                 # CodeAnalyzer trait + CodeEdge / ResolutionTarget types
│   ├── rust.rs                # use / call / type / impl queries
│   ├── typescript.rs          # ditto for TS / TSX
│   ├── python.rs
│   └── go.rs
├── markdown/                  # NEW — Markdown analyzer
│   ├── mod.rs                 # MarkdownAnalyzer entry point
│   ├── links.rs               # [text](path) extractor
│   ├── mentions.rs            # `code` → :Symbol resolver
│   ├── code_blocks.rs         # fenced-code path-header extractor
│   └── sections.rs            # heading → :DocSection
├── resolver/                  # NEW — symbol / artifact resolution
│   ├── mod.rs                 # SymbolResolver + ArtifactResolver
│   ├── module_map.rs          # crate → path mapping
│   ├── package_map.rs         # Cargo.toml / package.json deps
│   └── intra_doc.rs           # Rust intra-doc link parser
├── coalescer.rs               # MODIFIED — extended dedupe key
├── mapper.rs                  # MODIFIED — analyzer dispatch
├── schema.rs                  # MODIFIED — new constraints
└── worker.rs                  # MODIFIED — live re-extraction trigger

crates/cortex-storage/src/
└── external_repos.rs          # NEW — TOML loader for cross-repo SDK paths
```

`cortex-workers/Cargo.toml` gains `pulldown-cmark` (markdown
parser). The Tree-sitter grammar deps are already present (Rust,
TS, TSX, JS, Python, Go, Java, C, C++) — the analyzers reuse them
without adding new deps.

## 2. Code analyzer (Tree-sitter queries)

### 2.1 Rust

Three queries per file:

```scheme
;; use_decl.scm
(use_declaration (scoped_use_list (scoped_identifier) @import))
(use_declaration argument: (scoped_identifier) @import)
(use_declaration argument: (identifier) @import)

;; call_expr.scm
(call_expression function: (identifier) @callee)
(call_expression function: (field_expression field: (field_identifier) @callee))
(call_expression function: (scoped_identifier) @callee)

;; type_use.scm
(field_declaration type: (type_identifier) @typeuse)
(parameter type: (type_identifier) @typeuse)
(generic_type type: (type_identifier) @typeuse)

;; impl_block.scm
(impl_item trait: (type_identifier) @trait type: (type_identifier) @impl_for)
```

### 2.2 TypeScript

```scheme
;; import.scm
(import_statement source: (string) @src)
(import_specifier name: (identifier) @import_name)

;; call_expr.scm
(call_expression function: (identifier) @callee)
(call_expression function: (member_expression property: (property_identifier) @callee))

;; class_extend.scm
(class_declaration name: (type_identifier) @child
  (class_heritage (extends_clause (identifier) @parent)))
```

### 2.3 Python

```scheme
;; import.scm
(import_statement name: (dotted_name) @import)
(import_from_statement module_name: (dotted_name) @from_module)

;; call_expr.scm
(call function: (identifier) @callee)
(call function: (attribute attribute: (identifier) @callee))

;; class_inherit.scm
(class_definition name: (identifier) @child
  superclasses: (argument_list (identifier) @parent))
```

Each query yields `(captured_text, start_byte, end_byte)`. The
extractor maps `start_byte` to a line number and stores the line
in the edge's `source_line` prop.

### 2.4 Per-language extractor signature

```rust
pub trait CodeAnalyzer {
    fn language(&self) -> CodeLanguage;
    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge>;
}

pub struct CodeEdge {
    pub from_node: NodeRef,    // {label, natural_key}
    pub edge_type: EdgeType,   // CALLS | IMPORTS_FILE | USES_TYPE | …
    pub to_target: ResolutionTarget,  // raw target the resolver dereferences
    pub source_line: Option<u32>,
    pub kind: &'static str,    // "function_call", "type_use", "use_decl", …
}

pub enum ResolutionTarget {
    SymbolName(String),               // resolver tier 1+2
    ModulePath(Vec<String>),          // resolver tier 1+2 for use decls
    ExternalPackage { name: String }, // resolver tier 3
}
```

## 3. Markdown analyzer

Three passes per file via `pulldown-cmark`:

### 3.1 Link extraction

Walk events for `Event::Start(Tag::Link(_, dest, _))`. The
destination is parsed:

- Starts with `http://` / `https://` → external; emit
  `(:Artifact)-[:LINKS_TO_URL]->(:ExternalUrl)` (low priority).
- Relative path with `.md` extension → resolve relative to the
  source file's directory. If the resolved path exists in the
  workspace, emit `(:Artifact:doc src)-[:LINKS_TO]->(:Artifact:doc dst)`.
- Relative path with code extension (`.rs`, `.ts`, `.py`, …) →
  emit `(:Artifact:doc src)-[:DOCUMENTS]->(:Artifact dst)`.
- `#anchor` → emit `:LINKS_TO_SECTION` against the resolved
  `(path, anchor)` `:DocSection`.

### 3.2 Symbol mention extraction

Walk events for `Event::Code(token)`. Apply the disambiguation
rules from [03-target-graph.md §3](./03-target-graph.md#3-edge-class-b--docccode):

- length ≥ 3 chars
- token matches a known `:Symbol.qualified_name` OR `:Symbol.name`
  in the same repo (resolver tier 1)
- when scoped form (`crate::module::Sym`), match against the
  qualified-name index (tier 2)

Emits `(:Artifact:doc)-[:MENTIONS {confidence}]->(:Symbol)`.
Confidence = 1.0 for qualified-name matches, 0.7 for bare-name
matches, 0.0 for unmatched (dropped).

### 3.3 Fenced-code path header

Walk events for `Event::Start(Tag::CodeBlock(_))` followed by
`Event::Text(line)`. If the first line of the code block matches
`^//\s*(.+\.\w{1,8})$` (Rust-style) or `^#\s*(.+\.\w{1,8})$`
(Python-style), resolve the path relative to the workspace and
emit `(:Artifact:doc)-[:DESCRIBES_PATH]->(:Artifact)`.

### 3.4 Section extraction

Walk events for `Event::Start(Tag::Heading(level, …))`. For each
heading, build the GitHub-flavoured slug
(`lowercase + replace([' ', non-alphanum], '-')`). Emit
`:DocSection` keyed on `(doc_path, anchor)` with props
`{level, title}`. The parent edge `(:Artifact:doc)-[:CONTAINS]->(:DocSection)`
is implicit via shared key prefix.

## 4. Resolver

### 4.1 Module map

Built once at bootstrap time per workspace. For Rust:

1. Walk every `Cargo.toml` in the workspace.
2. For each crate, parse `[lib]` / `[[bin]]` to find the entry
   point file.
3. Walk the module tree: `mod foo;` declarations + `foo.rs` /
   `foo/mod.rs` resolution.
4. Build a map `{crate_name}::{module::path} → artifact_path`.

For TypeScript: walk `package.json` + `tsconfig.json` `paths`
mapping. For Python: walk `pyproject.toml` + `__init__.py` files.

The module map is a hot-reloaded resource — when a Cargo.toml or
mod.rs changes, the affected slice rebuilds.

### 4.2 Package map

Workspace deps that map to other workspace members are resolved
to local artifact paths. External crates (crates.io / npm / pypi)
get an `:ExternalPackage` node.

For HiveLLM-internal SDKs (vectorizer-sdk → e:/HiveLLM/Vectorizer/),
the operator declares the path in a new
`crates/cortex-storage/src/external_repos.toml` file:

```toml
[vectorizer-sdk]
local_path = "../Vectorizer"
crate_name = "vectorizer"

[nexus-graph-sdk]
local_path = "../Nexus"
crate_name = "nexus_graph"
```

When `local_path` is set, the resolver promotes the edge from
`:IMPORTS_EXTERNAL` to `:IMPORTS_FILE` (cross-repo) by walking the
local clone's module map.

### 4.3 Symbol resolver

Three tiers (in order):

1. **Same-file symbol table** — every CodeAnalyzer pass produces a
   per-file `local_symbols: BTreeMap<String, NaturalKey>`. Tier-1
   resolves bare-name calls against this table first.
2. **Crate symbol index** — built incrementally as each file's
   tier-1 pass completes. Maps `(repo, crate, qualified_name) →
   :Symbol natural_key`. Tier-2 resolves scoped names.
3. **External / unresolved** — fallback to `:ExternalPackage` (when
   the import path matches a declared dep) or `:UNRESOLVED_IMPORT`
   (when nothing matches).

## 5. Triggering

### 5.1 Bootstrap pass

`cortex-bootstrap` (existing CLI under
[`crates/cortex-cli/src/bin/cortex-bootstrap.rs`](../../../crates/cortex-cli/src/bin/cortex-bootstrap.rs))
gains a new `--graph-static` flag. When set, after the existing
artifact pass, it walks the workspace once more and runs the code +
markdown analyzers, accumulating edges into the same archive sink
the structural events use. The graph worker re-reads the archive
at boot (phase11i §5.2 archive_loader) and applies the new edges.

### 5.2 Live pass

The graph worker
([`crates/cortex-workers/src/graph/worker.rs`](../../../crates/cortex-workers/src/graph/worker.rs))
intercepts `Edit` / `Write` / `MultiEdit` tool_calls. For each
TouchedArtifact, it pulls the new content via the existing
content-hash addressed CAS, runs the analyzer, and emits the
resulting GraphPatch alongside the structural one.

Markdown analyzer fires on the same trigger when the touched path
ends with `.md`.

### 5.3 Idempotency

Every emitted edge carries `(from_natural_key, edge_type,
to_natural_key, source_event_id)`. Re-running against the same
content_hash produces the same set of edges. Re-emit is a no-op at
the Cypher MERGE layer.

When content_hash changes (the file was edited), the analyzer
re-runs and emits the new edge set. Stale edges from the previous
content_hash get cleaned up by a nightly sweep keyed on
`(source_event_id outdated, ttl_days = 30)` so a deleted symbol's
edges fade out without leaving dangling pointers.

## 6. Cost budget

Per-file extraction:

| Step                          | Cost (approx)             |
| ----------------------------- | ------------------------- |
| Tree-sitter parse + 3 queries | 0.3 ms / 100 LOC          |
| Markdown link extraction      | 0.1 ms / file             |
| Symbol resolver tier 1        | O(1) — local hashmap      |
| Symbol resolver tier 2        | O(log N) — sorted index   |
| Symbol resolver tier 3        | O(1) — pre-built dep map  |

For the cortex repo (~120 K LOC, ~80 markdown files), bootstrap
adds ~400 ms total. Live per-edit adds ~5-10 ms per touched file,
under the existing 100 ms graph-write budget.

## 7. Output volume

Estimated edges produced per Artifact:

| Artifact kind | Edges before | Edges after |
| ------------- | ------------ | ----------- |
| Code (Rust)   | ~2 (IN_REPO + DEFINES per symbol) | ~10-15 (+ IMPORTS_FILE × 5 + CALLS × 6 + USES_TYPE × 3) |
| Code (TS)     | ~2           | ~12         |
| Spec (.md)    | ~1 (IN_REPO) | ~8 (+ MENTIONS × 5 + LINKS_TO × 2) |
| ADR (.md)     | ~1           | ~6 (+ CITES × 4)                      |

Across the cortex corpus (~3 K artifacts), total new edges land
around **35 000-45 000**. Nexus handles a graph of that size in
~80 MB memory; query latency stays under the spec-11 budget.

## 8. Failure-mode handling

| Failure                         | Behaviour                                  |
| ------------------------------- | ------------------------------------------ |
| Tree-sitter parser crash        | Per-file fallback: emit zero code edges, log WARN with file path. The structural skeleton is unaffected. |
| Markdown parser exception       | Same.                                      |
| Resolver tier-3 ambiguity       | Emit `:UNRESOLVED_IMPORT` with raw target. |
| Stale module map after Cargo.toml edit | Best-effort: stale-cached map can resolve a removed crate. Nightly sweep cleans up. |
| Cross-repo external_repos.toml missing | External imports still emit, just go to `:ExternalPackage` instead of `:IMPORTS_FILE` cross-repo. Soft-degrade. |

## 9. Schema migrations

The new node labels + constraints land via additive Cypher
statements in `schema.rs`. Existing constraints are unchanged. A
fresh boot applies the new statements unconditionally; an upgrade
boot applies them via `IF NOT EXISTS` (already the pattern in the
existing schema bootstrap), making the rollout idempotent.

## 10. What this does NOT replace

- The Sonnet semantic layer (`emit_classifier_entities`) keeps
  firing. Its high-level relations (`DISCUSSES`, `FIXES`,
  `IMPLEMENTS` at the conceptual level) catch what static
  extraction cannot — *intent*, not just structure. The two
  layers are complementary.
- The structural skeleton (HAS_TURN, TOUCHED, IN_REPO) stays
  identical. New edges sit alongside.
- Vector + keyword retrieval — the graph layer adds traversable
  context to the bundle; it does not replace the lane-fusion
  ranker. A `pre_change_context` query still runs the three lanes,
  fuses them, AND walks the graph 1-2 hops from the top hits.
