# Understand-Anything — Findings

Numbered, evidenced findings. Each: what UA does, the evidence, and the Cortex relevance.

---

## F-1 — Incremental graph patching via git-hash fingerprint

**What:** UA never re-analyzes a whole repo on change. `staleness.ts` stores `lastCommitHash`
(`gitCommitHash` in `meta.json`); `isStale()` runs `git diff <lastCommitHash>..HEAD --name-only`
to get changed files; `mergeGraphUpdate()` removes nodes whose `filePath` is in the changed set,
prunes edges with a dangling source/target, appends freshly analyzed nodes/edges, then stamps the
new commit hash + `analyzedAt` timestamp.

**Evidence:** `packages/core/src/staleness.ts` — `getChangedFiles()`, `mergeGraphUpdate()`.

**Cortex relevance — HIGH.** Cortex's graph/embedding indexer should patch, not rebuild. Map
`filePath → node-id set` so a commit touching 3 files re-embeds 3 files' worth of nodes, not the
repo. The git-diff-since-last-hash pattern is the cheapest possible staleness oracle and Cortex
already has git context in scope derivation.

---

## F-2 — Tiered change classifier gates downstream re-work

**What:** `change-classifier.ts:classifyUpdate()` aggregates
`structuralCount = structurallyChangedFiles + newFiles + deletedFiles` and buckets the commit:

| Bucket | Trigger | Effect |
|--------|---------|--------|
| `SKIP` | all files NONE or COSMETIC | no graph work |
| `PARTIAL_UPDATE` | localized structural change, no dir reorg | re-analyze changed files only |
| `ARCHITECTURE_UPDATE` | new/removed directories **or** >10 structural files | `rerunArchitecture: true`, `rerunTour: true` |
| `FULL_UPDATE` | >30 structural files **or** >50% of project | full re-analysis |

Cosmetic-only diffs (whitespace, comments) classify as SKIP and cost nothing.

**Evidence:** `packages/core/src/change-classifier.ts`.

**Cortex relevance — MEDIUM/HIGH.** Cortex consolidation + graph re-work should be budget-tiered
the same way: a comment-only commit shouldn't trigger consolidation; a directory reshuffle should
invalidate architecture-level summaries / topic cards. This is a re-work governor that maps onto
Cortex's existing consolidation scheduler.

---

## F-3 — Two-phase analyzer with hard anti-hallucination contract

**What:** `file-analyzer` agent runs **Phase 1 deterministic** (`extract-structure.mjs`:
tree-sitter for 10 code langs + 12 non-code parsers → functions, classes, imports, exports, call
graph, metrics) then **Phase 2 semantic** (LLM consumes the structured facts → summaries,
complexity, semantic edges). Explicit rules:

- "NEVER invent file paths. Every `filePath` and node ID must correspond to a real file."
- "Import edges must enumerate ALL imports from `batchImportData`; output count must equal input count."
- Significance filter: functions/classes emitted only if ≥10 lines **or** exported.
- "Do NOT re-read source files unless the script skipped a file."
- Strict ID prefixes: `file:<path>`, `function:<path>:<name>`, `class:<path>:<name>`.

**Evidence:** `agents/file-analyzer.md`.

**Cortex relevance — HIGH.** The "LLM may only describe facts the deterministic pass produced,
and the count must reconcile" contract is the single most portable anti-hallucination pattern.
Cortex's adapter/workers that emit graph edges should adopt: deterministic extractor produces the
fact set, LLM annotates, a reconciliation check rejects any edge whose endpoints aren't in the
fact set. Directly strengthens graph trustworthiness.

---

## F-4 — Rich node/edge ontology (21 node types, 35 edge types)

**Node types (21):**
- Code (5): `file`, `function`, `class`, `module`, `concept`
- Non-code (8): `config`, `document`, `service`, `table`, `endpoint`, `pipeline`, `schema`, `resource`
- Domain (3): `domain`, `flow`, `step`
- Knowledge (5): `article`, `entity`, `topic`, `claim`, `source`

**Edge types (35):**
- Structural: `imports`, `exports`, `contains`, `inherits`, `implements`
- Behavioral: `calls`, `subscribes`, `publishes`, `middleware`
- Data flow: `reads_from`, `writes_to`, `transforms`, `validates`
- Dependencies: `depends_on`, `tested_by`, `configures`
- Semantic: `related`, `similar_to`
- Infra: `deploys`, `serves`, `provisions`, `triggers`
- Schema/data: `migrates`, `documents`, `routes`, `defines_schema`
- Domain: `contains_flow`, `flow_step`, `cross_domain`
- Knowledge: `cites`, `contradicts`, `builds_on`, `exemplifies`, `categorized_under`, `authored_by`

**Core interfaces (verbatim):**

```typescript
interface GraphNode {
  id: string;
  type: NodeType;
  name: string;
  filePath?: string;
  lineRange?: [number, number];
  summary: string;
  tags: string[];
  complexity: "simple" | "moderate" | "complex";
  languageNotes?: string;
  domainMeta?: DomainMeta;
  knowledgeMeta?: KnowledgeMeta;
}
interface GraphEdge {
  source: string; target: string; type: EdgeType;
  direction: "forward" | "backward" | "bidirectional";
  description?: string; weight: number;
}
interface KnowledgeGraph {
  version: string;
  kind?: "codebase" | "knowledge";
  project: ProjectMeta;
  nodes: GraphNode[]; edges: GraphEdge[];
  layers: Layer[]; tour: TourStep[];
}
```

**Evidence:** `packages/core/src/types.ts`.

**Cortex relevance — HIGH.** This is a superset of Cortex's current relations
(`IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES`). The **knowledge** sub-ontology
(`claim`/`entity`/`topic`/`source` + `contradicts`/`builds_on`/`cites`) maps almost 1:1 onto
Cortex's topic-card / contradiction-detection design already described in the pre-thinking
analysis. Adopt the edge taxonomy as the Nexus relation vocabulary; the weighted, directional
edge with optional `description` is a good Nexus edge shape.

---

## F-5 — Karpathy-pattern knowledge-base graph

**What:** `/understand-knowledge` + `article-analyzer` agent parse a wiki: extract wikilinks and
categories from `index.md`, then LLM agents "discover implicit relationships, extract entities,
and surface claims." Produces `article`/`entity`/`topic`/`claim`/`source` nodes connected by
`cites`/`contradicts`/`builds_on`/`exemplifies`/`categorized_under`/`authored_by`.

**Evidence:** README "Knowledge Base Analysis"; `agents/article-analyzer.md`; node/edge taxonomy.

**Cortex relevance — MEDIUM.** Cortex explicitly follows Karpathy editing discipline and already
has topic cards + contradiction detection. UA gives a concrete *graph materialization* of the
same idea: claims as first-class nodes with `contradicts`/`builds_on` edges enables
contradiction-aware retrieval at the graph layer, not just at consolidation. Candidate for the
docs/decisions lane.

---

## F-6 — 12 non-code parsers extend the graph past source

**What:** deterministic parsers for `dockerfile`, `env`, `graphql`, `json`, `makefile`,
`markdown`, `protobuf`, `shell`, `sql`, `terraform`, `toml`, `yaml` feed the same node/edge
schema (→ `config`/`schema`/`table`/`endpoint`/`resource`/`pipeline` nodes,
`deploys`/`provisions`/`defines_schema`/`migrates`/`routes` edges).

**Evidence:** `packages/core/src/plugins/parsers/*.ts`.

**Cortex relevance — MEDIUM.** Cortex graphs sessions/decisions/code; infra+data files are a blind
spot. SQL → `table`/`defines_schema`, Terraform → `resource`/`provisions`, protobuf/GraphQL →
`schema`/`endpoint` would let Cortex answer "what touches table X / service Y" — high value for a
multi-repo ecosystem (Vectorizer, Nexus, Synap…). Pluggable-parser registry is the portable shape.

---

## F-7 — Auto-update hooks (git-commit + session-start)

**What:** `hooks.json` wires two events:
- **PostToolUse(Bash)** — regex-matches `git (commit|merge|cherry-pick|rebase)`, and if
  `autoUpdate:true` + graph exists, emits a directive forcing the assistant to run
  `auto-update-prompt.md` ("Do not ask the user for confirmation — just do it.").
- **SessionStart** — if stored `gitCommitHash != git rev-parse HEAD`, emits the same staleness
  directive.

**Evidence:** `hooks/hooks.json` (quoted verbatim in [adoption.md](07-adoption.md)).

**Cortex relevance — LOW/MEDIUM.** Cortex already uses SessionStart + PreToolUse hooks. The
borrow here is the *pattern*: detect staleness cheaply at session start and after commits, then
push a self-executing directive rather than asking. Cortex's daemon already captures commits, so
this is mostly confirmation Cortex is on the right track; the cheap `meta.json` hash compare is a
nice fallback for when the daemon is down (cf. known pipeline-recovery gotchas).

---

## F-8 — Persona-aware, dependency-ordered guided tours

**What:** `tour-builder` emits `tour: TourStep[]` — a dependency-ordered walkthrough; detail
level adapts to persona (junior dev / PM / power user).

**Evidence:** README "Guided Tours"; `KnowledgeGraph.tour`.

**Cortex relevance — LOW.** Not core to Cortex's memory mission, but a dependency-ordered tour
over the graph is a plausible *output* for Cortex's GUI timeline/branch views (onboarding mode).
Park as a GUI idea, not a near-term borrow.

---

## F-9 — Diff impact analysis

**What:** `/understand-diff` shows which graph regions a pending change affects before commit, by
walking edges out from changed nodes.

**Evidence:** README "Diff Impact Analysis".

**Cortex relevance — MEDIUM.** Cortex's pre-thinking already expands the graph 1–2 hops from seed
artifacts. "Given this diff, what decisions/laws/sessions are blast-radius-adjacent" is a natural
Cortex pre-thinking enrichment: seed = changed files, expand via `imports`/`depends_on`/`tested_by`,
surface attached decisions/laws. Reuses existing graph-expansion machinery.
