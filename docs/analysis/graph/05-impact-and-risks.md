# 05 — Impact & risks

## 1. Quantitative impact

### 1.1 Graph density

| Metric                           | Today              | After phase11k     | Lift     |
| -------------------------------- | ------------------ | ------------------ | -------- |
| Total edges (cortex repo, est.)  | ~5 K               | ~50 K              | **10×**  |
| Edges per Artifact (code, avg)   | 2                  | ~12                | **6×**   |
| Edges per Artifact (doc, avg)    | 1                  | ~8                 | **8×**   |
| Edges between artifacts          | ~150 (TOUCHED via tool calls) | ~12 K (IMPORTS_FILE + DOCUMENTS + LINKS_TO) | **80×** |
| Cross-repo edges                 | 0                  | ~600               | ∞        |
| Connected-component count        | ~80 (one per session) | ~5 (corpus is genuinely connected) | -16× (better) |

The component-count drop is the most operator-visible win. Today
the graph is a forest of session islands; after phase11k it's
genuinely interconnected and `match (a)-[*1..3]-(b)` returns
useful neighbours instead of just same-session siblings.

### 1.2 Query bundle

Per-intent expected lift on the relevance gold-set
([phase11i §4.4](../../../crates/cortex-api/tests/fixtures/relevance-gold.json),
30 questions):

| Intent                | MRR@10 today | MRR@10 target | Mechanism                                  |
| --------------------- | ------------ | ------------- | ------------------------------------------ |
| `pre_change_context`  | ~0.86        | ~0.92         | Symbol-call traversal surfaces caller files |
| `decision_lookup`     | ~0.70        | ~0.90         | ADR → Spec → Code citation chain           |
| `similar_problems`    | ~0.83        | ~0.88         | Doc-mentions tighten the topic cluster     |
| `law_check`           | ~0.67        | ~0.85         | Law → AGENTS.md → spec citations traversable |
| `free_search`         | ~0.75        | ~0.85         | Doc-anchored questions land on the doc + the file at once |

Aggregate MRR@10 lift: **+0.07** absolute, holds the §4.5 IT gate
(0.75 floor) with substantial headroom. NDCG@10 lifts more (≈ +0.09)
because the new edges push the right hit toward rank 1.

### 1.3 Storage / runtime

| Metric                           | Today     | After phase11k | Delta    |
| -------------------------------- | --------- | -------------- | -------- |
| Nexus DB size (cortex repo)      | ~10 MB    | ~85 MB         | +75 MB   |
| Bootstrap time                   | ~120 s    | ~125 s         | +4 %     |
| Per-edit graph-write latency     | ~30 ms    | ~38 ms         | +8 ms    |
| Per-event Sonnet cost            | ~$0.0003  | unchanged      | n/a      |
| Static-extraction cost / month   | $0        | $0             | $0       |

Static extraction is free at runtime. The Sonnet semantic layer
keeps firing (it still catches intent-level edges that no
syntactic analysis sees), so per-event cost is unchanged.

## 2. Risk register

### 2.1 Resolver false positives

**Risk:** Symbol-mention resolver attaches a `:MENTIONS` edge from a
doc to the wrong symbol when the bare name (`fuse`, `run`, `parse`)
matches multiple `:Symbol`s across the repo.

**Mitigation:** Three-tier disambiguation
([04-extraction-pipeline.md §3.2](./04-extraction-pipeline.md#32-symbol-mention-extraction)).
Bare-name resolution gets `confidence < 1.0` so the renderer can
filter on `confidence ≥ 0.9` for high-precision queries. The doc
mention is preserved alongside the qualified-name target where
available; the dashboard surfaces the lower-confidence edges
under a "fuzzy" affordance.

**Acceptance bar:** spot-check 50 random `:MENTIONS` edges; ≤ 5 %
false-positive rate. IT lands as `mentions_precision_it.rs` under
the new crate.

### 2.2 Tree-sitter grammar coverage holes

**Risk:** A new language lands in the corpus (Kotlin, Swift,
Lua, …) and the analyzer silently emits zero edges instead of
fewer.

**Mitigation:** Per-language analyzer fails closed: when no
analyzer matches the file's extension, emit a warning event
`graph.unsupported_language` with the path. Operators see the
gap in the dashboard's coverage panel. Adding a new language is
one CodeAnalyzer impl + one Tree-sitter dep.

### 2.3 Macro-generated code (Rust)

**Risk:** `derive(Serialize)` / `tokio::main` / custom proc-macros
generate calls and types invisible to syntactic analysis. Edge
recall on heavily macro-using files drops below 50 %.

**Mitigation:** Document the limitation in operator docs. The
Sonnet semantic layer covers the gap for the most-cited macros
(`tokio::spawn`, `tracing::instrument`). A future phase can wire
the rust-analyzer LSP for full macro expansion if the loss
matters; until then the static layer's recall ceiling on
macro-heavy files is acknowledged.

### 2.4 Broken-link decay

**Risk:** A markdown link `[fix me](crates/old-path/file.rs)` lands
the `:DOCUMENTS` edge against an artifact that no longer exists
(the file was renamed or deleted).

**Mitigation:** The resolver checks the target exists in the
workspace at extraction time. Missing targets emit
`:UNRESOLVED_DOC_LINK` with the raw destination so the dashboard
flags the broken link without polluting the resolved-edge counts.
Nightly sweep promotes broken links to surface in a
`graph.broken_links_count` health metric.

### 2.5 Re-extraction churn

**Risk:** Every Edit/Write tool_call re-runs the analyzer on the
touched file. A high-frequency editing burst (an agent's
search-and-replace pass) triggers hundreds of redundant
re-extractions.

**Mitigation:** Coalesce by content_hash. The graph worker
deduplicates `(content_hash, analyzer_version)` patches per
session — the second edit of the same file with the same
resulting content_hash is a no-op. Pre-existing graph patch
coalescer already exists for the structural skeleton; extend it
to cover the new patches.

### 2.6 Cross-repo SDK drift

**Risk:** `external_repos.toml` declares `vectorizer-sdk → ../Vectorizer`
but the operator's Vectorizer checkout is a different version
than the one cortex-api compiles against. Cross-repo edges point
at symbols that may not exist in the compiled-against version.

**Mitigation:** The edge prop carries `extracted_at = ts` + the
local checkout's git sha. The dashboard surfaces a `:VERSION_DRIFT`
warning when the version field on the `:ExternalPackage` node
differs from the local checkout's manifest version. Drift is
visible, not silent.

### 2.7 Privacy / leakage

**Risk:** Extracting symbol mentions from `.rulebook/learnings/`
docs surfaces internal symbol names in the dashboard's `:MENTIONS`
edge view, potentially exposing names from private repos to a
shared dashboard.

**Mitigation:** The redactor (cortex-core/src/redact.rs) runs
BEFORE the markdown analyzer pass. Symbols mentioned inside
`[REDACTED:*]` markers stay redacted. The analyzer reads the
redacted body, never the raw payload. ACL on the dashboard's
graph view (existing in phase3 §7) restricts cross-repo
:MENTIONS edges to operators with the matching repo scope.

### 2.8 Performance regression at scale

**Risk:** A 1 M-LOC corpus (UzEngine-scale) produces ~10 M edges,
which exceeds Nexus's comfortable single-node ceiling.

**Mitigation:** The schema is sharded by `repo` (every Artifact
edge anchors at `IN_REPO`). Per-repo Nexus instances are
addressable today (the storage layer's `cortex-{slug}-graph`
naming convention). The dashboard's traversal uses per-repo
scoping by default; cross-repo traversals run against the
explicit cross-repo subset. Path: graph sharding lands in
phase11l if/when the corpus crosses the 5 M-edge threshold.

## 3. Success criteria

The phase is successful when:

1. **Coverage** — ≥ 90 % of Rust files in the cortex corpus emit
   IMPORTS_FILE + CALLS + USES_TYPE edges. ≥ 80 % of TS / Py files
   emit equivalents.
2. **Doc anchoring** — ≥ 95 % of files under `docs/specs/` and
   `.rulebook/decisions/` have at least one outbound :DOCUMENTS,
   :MENTIONS, or :CITES edge.
3. **Relevance lift** — gold-set MRR@10 (phase11i §4.5) lifts by
   ≥ +0.05 absolute against the post-phase11i baseline.
4. **Idempotent re-runs** — re-running bootstrap against an
   already-populated graph is a no-op (zero new edges, zero
   updated props).
5. **Graceful degradation** — a missing Tree-sitter grammar, a
   broken markdown file, or a missing external_repos.toml entry
   produces a per-file warning + zero edges from that file
   without affecting any other file's extraction.
6. **No structural regression** — the existing 13 edge types from
   [01-current-state.md §3](./01-current-state.md#3-edge-types-emitted-today)
   continue to land identically. Existing graph queries return the
   same results within float epsilon.

## 4. Out of scope

- **Live LSP-mediated extraction** — using rust-analyzer / tsserver
  for full type-aware resolution. Defer to a later phase if the
  syntactic resolver's ~10 % wrong-target rate matters more than
  expected.
- **Embedding the graph** — node embeddings via Node2Vec / DeepWalk
  for graph-aware vector search. Separate concern.
- **Cypher query templates for the new edges** — the orchestrator
  picks up the new edges via existing `pre_change_context` /
  `decision_lookup` strategies, but the graph-lane's Cypher
  template registry will need new templates for "all callers" /
  "doc trail" queries. Lands in phase11l alongside the renderer
  uplift that surfaces the new edges in the spec-12 bundle.
