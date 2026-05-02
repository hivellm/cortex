# 01 — Current state of the Cortex graph layer

## 1. Code surface (where edges come from)

The graph layer lives under
[`crates/cortex-workers/src/graph/`](../../../crates/cortex-workers/src/graph/).
The schema is bootstrapped by
[`schema.rs`](../../../crates/cortex-workers/src/graph/schema.rs) and
patches are produced by
[`mapper.rs::map_event_to_patch`](../../../crates/cortex-workers/src/graph/mapper.rs#L85).

Every event is mapped through two passes:

1. **Structural pass** (`emit_*` per kind) — produces the canonical
   skeleton from envelope metadata alone. Pure, deterministic,
   idempotent under replay.
2. **Sonnet semantic pass** (`emit_classifier_entities`) — reads
   `event.classifier.entities` + `event.classifier.relations` and
   anchors each relation at the current event's primary node.

## 2. Schema bootstrap (every node label that has a uniqueness constraint)

From [`schema.rs:20`](../../../crates/cortex-workers/src/graph/schema.rs#L20):

| Label          | Natural key       | Constraint                          |
| -------------- | ----------------- | ----------------------------------- |
| `Session`      | `id`              | UNIQUE                              |
| `Turn`         | `id`              | UNIQUE                              |
| `ToolCall`     | `id`              | UNIQUE                              |
| `Artifact`     | `natural_key`     | UNIQUE (`repo|path|content_hash`)   |
| `Decision`     | `id`              | UNIQUE                              |
| `Memory`       | `id`              | UNIQUE                              |
| `Analysis`     | `id`              | UNIQUE                              |
| `Law`          | `id`              | UNIQUE                              |
| `LawViolation` | `id`              | UNIQUE                              |
| `Repo`         | `name`            | UNIQUE                              |
| `Symbol`       | `natural_key`     | UNIQUE (`repo|lang|qualified_name`) |

Indexes:
- `(:Artifact).(repo, path)`
- `(:Turn).(ts)`
- `(:ToolCall).(tool_name)`
- `(:Symbol).(repo, name)`

Notable absences:
- `:Knowledge`, `:Learning`, `:Consolidation` (phase11j) — emit through
  the `Memory` constraint via `emit_memory`. No dedicated label
  constraint; the dashboard's colour-coded view reads `entity_type`
  prop instead.
- `:Spec`, `:Analysis-doc`, `:CodeSymbol`, `:Topic` — no label,
  treated as plain `Artifact` or `Concept` nodes.

## 3. Edge types emitted today

Grep over [`mapper.rs`](../../../crates/cortex-workers/src/graph/mapper.rs)
for `edge_type` (the per-edge label assignment). There are exactly
**13 distinct edge types** in production:

### Structural (always emitted from envelope metadata)

| Edge                                                | Source | Sink     | Trigger                                       |
| --------------------------------------------------- | ------ | -------- | --------------------------------------------- |
| `(:Session)-[:HAS_TURN]->(:Turn)`                   | Turn   | Turn     | every Turn event                              |
| `(:Turn)-[:HAS_TOOL_CALL]->(:ToolCall)`             | Turn   | ToolCall | every ToolCall event with parent turn         |
| `(:Session)-[:HAS_TOOL_CALL]->(:ToolCall)`          | Sess.  | ToolCall | ToolCall without a parent turn (orphan)       |
| `(:ToolCall)-[:TOUCHED {operation}]->(:Artifact)`   | TC     | Artifact | per `payload.touched_artifacts[]`             |
| `(:Symbol)-[:DEFINES]->(:Artifact)`                 | Sym.   | Artifact | per code symbol from Tree-sitter chunker      |
| `(:Artifact)-[:IN_REPO]->(:Repo)`                   | Art.   | Repo     | every Artifact event with `context.repo` set  |
| `(:Session)-[:REMEMBERS]->(:Memory)`                | Sess.  | Memory   | every Memory / Knowledge / Learning / Consolidation event |
| `(:Turn)-[:LINKED_TO {role}]->(:Decision)`          | Turn   | Decision | when a Decision payload references a turn     |
| `(:Decision)-[:SUPERSEDES]->(:Decision)`            | Dec.   | Decision | when `payload.supersedes` is set              |
| `(:Analysis)-[:ANALYZES]->(:Artifact|:Decision|…)`  | An.    | varies   | per Analysis target                           |
| `(:LawViolation)-[:OF]->(:Law)`                     | LV     | Law      | every LawViolation                            |
| `(:LawViolation)-[:OBSERVED_IN]->(:Turn|:ToolCall)` | LV     | event    | per `payload.observed_in`                     |

### Semantic (Sonnet-extracted, optional)

`emit_classifier_entities` walks `event.classifier.relations`. Allowed
labels (from
[`mapper.rs::normalise_relation_label`](../../../crates/cortex-workers/src/graph/mapper.rs#L303)):

```rust
"REFERENCES" | "IMPLEMENTS" | "FIXES" | "DISCUSSES" | "DEFINES"
| "DEPENDS_ON" | "SUPERSEDES" | "OBSERVED_IN" | "TOUCHED"
```

Every Sonnet-emitted edge is anchored at the current event's primary
node — so `(:Turn)-[:DISCUSSES]->(:Decision)`, `(:ToolCall)-[:FIXES]->(:Bug)`,
etc. Source quality varies wildly:

- **Hit rate**: per-event classification budget caps Sonnet calls,
  so most events fall through to the static fallback which produces
  **no entities + no relations** ([cortex-classifier statics.rs](../../../crates/cortex-classifier/src/statics.rs)
  has zero `entities.push(…)` / `relations.push(…)` calls).
- **Recall**: even on Sonnet-classified events, the prompt's
  controlled vocabulary is small (10 entity types, 9 relation
  types) and the per-event window is one envelope. The model
  doesn't see the file's import list or the markdown link block.

In practice the semantic layer covers ~5-10 % of events with ~1-3
edges each. Decoration, not infrastructure.

## 4. What the structural skeleton actually looks like

For a typical 20-turn coding session that touches 3 files:

```text
Session (1)
  ├─[HAS_TURN]─→ Turn (20)
  │   └─[HAS_TOOL_CALL]─→ ToolCall (~60)
  │       └─[TOUCHED]─→ Artifact (~3 unique)
  │           ├─←[DEFINES]─ Symbol (~30 per file)
  │           └─[IN_REPO]─→ Repo (1)
  └─[REMEMBERS]─→ Memory (~5 if any rulebook captures fired)
```

Total nodes: ~150. Total edges: ~250. Of those edges, **0** connect
the corpus *across* sessions — every traversal hits the Session root
and stops. There is no path from "today's edit on
`crates/cortex-api/src/fusion.rs::rrf_fuse`" to "yesterday's edit on
the same symbol from a different session", because:

- The `Artifact` node is keyed on `(repo, path, content_hash)`. Two
  sessions editing the same file produce TWO Artifact nodes
  (different content_hash).
- The `Symbol` node IS shared (keyed on `(repo, lang, qualified_name)`),
  but no edge connects sessions through symbols. `DEFINES` points
  Symbol→Artifact, not Symbol→Turn/ToolCall.

## 5. What's NOT in mapper.rs

A grep for terms the user expected to see in a useful graph:

```bash
$ grep -E "IMPORTS|CALLS|USES_TYPE|EXTENDS|MENTIONS|LINKS_TO|CITES|DOCUMENTED_BY" mapper.rs
# (no matches)
```

The chunker (`crates/cortex-workers/src/embedder/chunker_code.rs`)
loads Tree-sitter grammars for Rust / TS / TSX / JS / Python / Go /
Java / C / C++ — but only walks the CST for top-level declarations to
populate `Symbol` nodes. The `use_declaration` / `import_statement` /
`call_expression` / `field_access` nodes the same parser exposes are
**never visited**. The CST is built and discarded.

Markdown is treated as plain text. There is no `pulldown-cmark` /
`comrak` / regex pass that extracts `[text](path/to/file.md)` link
targets, `` `crates/cortex-api/src/fusion.rs::FusionConfig` `` symbol
mentions, or fenced-code-block file headers.

ADR / Decision payloads carry a `links: Vec<String>` field
([events.rs::DecisionPayload](../../../crates/cortex-core/src/events.rs))
but no graph emitter resolves those links to `:Artifact` or
`:Decision` nodes. They live as a free-form prop bag on the Decision
node and never become traversable edges.

## 6. Summary

The graph layer today gives the agent **session-level** structure
(this turn touched these files) and **decoration** (occasional Sonnet
hints). It does NOT give:

- intra-file structure (this function calls that function)
- intra-repo structure (this file imports that module)
- cross-repo structure (this code uses Vectorizer SDK)
- doc-anchored structure (this spec describes that file / symbol)
- ADR / decision provenance (this decision cites this spec section)

Phase11j's consolidation tier compresses the noisy turn stream into
curated summaries, but consolidations themselves connect to nothing
beyond their `source_event_ids[]`. The relevance ceiling is set by
how richly the graph encodes structural reality, not by how many
turns we ingest.

The next file ([02-gaps.md](./02-gaps.md)) catalogs the gaps by
failure mode — what concrete query falls flat for each missing edge
class.
