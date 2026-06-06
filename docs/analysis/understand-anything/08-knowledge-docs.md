# Documentation as a Graph — Knowledge-Base Understanding for Cortex

Expands the analysis beyond code: how UA turns **docs / wikis / second-brain notes** into a graph,
and how Cortex should adopt it. Source: `agents/article-analyzer.md`,
`docs/superpowers/specs/2026-04-09-understand-knowledge-design.md`, `types.ts` knowledge ontology.
See also [02-findings.md](02-findings.md) F-5.

---

## 1. In plain terms

UA runs the *same idea* on documentation that it runs on code: a deterministic pass extracts what's
literally there, then an LLM pass adds meaning under tight guard. For docs the "facts" are:
markdown files, their frontmatter, their `[[wikilinks]]`, their headings and categories. The LLM
then surfaces what the links *don't* say — the **claims** an article makes, the **entities** it
mentions, and whether it **agrees, extends, or contradicts** other docs.

The payoff for Cortex: documentation stops being a flat pile of text the model fuzzy-matches
against, and becomes a queryable web where *"this note contradicts that ADR"* is a first-class fact,
not something the model has to notice by luck.

---

## 2. UA's knowledge pipeline (5 stages)

```
knowledge-scanner → format-detector → article-analyzer → relationship-builder → graph-reviewer
```

| Stage | Deterministic? | Does |
|-------|----------------|------|
| `knowledge-scanner` | yes | inventories all markdown files |
| `format-detector` | yes | detects Obsidian (`.obsidian/`), Logseq (`logseq/`+`pages/`), Dendron… by signature files; picks the right wikilink/frontmatter/block-ref parser |
| `article-analyzer` | LLM (gated) | per-file: emits `entity` + `claim` nodes + implicit edges with textual evidence |
| `relationship-builder` | LLM ("heavy step") | cross-file: discovers `builds_on`/`contradicts`/`categorized_under`, clusters articles into `topic` nodes |
| `graph-reviewer` | yes | referential-integrity validation |

The graph is tagged `"kind": "knowledge"` (vs `"codebase"`) — same schema, different node/edge
emphasis and dashboard layout.

---

## 3. The knowledge ontology (already in [03-ontology-mapping.md](03-ontology-mapping.md))

**Nodes (5):** `article`, `entity`, `topic`, `claim`, `source`.
**Edges (6):** `cites`, `contradicts`, `builds_on`, `exemplifies`, `categorized_under`, `authored_by`.

`KnowledgeMeta` (optional node metadata): detected format, wikilinks, backlinks, frontmatter,
source URLs, and a **confidence score (0–1)** for inferred relationships.

---

## 4. The same anti-invention contract applies (key for trust)

UA's `article-analyzer` carries the doc-equivalent of the code reconciliation gate
([05-extraction-contract.md](05-extraction-contract.md)):

- **Deterministic first:** the parser already created `related` edges from `[[wikilinks]]`.
  The LLM must **NOT duplicate wikilink edges** — only surface what links miss.
- **"Be conservative":** an edge is emitted only with **explicit textual evidence**; thematic
  similarity alone is insufficient (this is what keeps `contradicts` honest).
- **Deduplicate entities:** one node per person/tool/paper across the batch; reference the existing
  node-id list rather than minting duplicates.
- **Bounded output:** for 10–15 articles → ~5–15 entities, ~5–10 claims, ~10–20 edges. Keeps the
  LLM from over-generating noise.
- **Every edge carries a `description`** = the evidence span. Auditable.

Edge weights encode confidence: `contradicts` 0.9, `builds_on` 0.8, `exemplifies`/`cites` 0.7,
`authored_by` 0.6.

---

## 5. Why this is a strong fit for Cortex specifically

Cortex already wants exactly this — the `phase11r_topic_card_mcp_enrichment` proposal explicitly
names "Karpathy LLM Wiki, Obsidian + Claude MCP" as the target second-brain contract:

1. **Topic surface the model rewrites when new evidence lands** → UA's `topic` nodes +
   `relationship-builder` clustering are the graph backing for Cortex's living topic cards.
2. **Contradictions surfaced explicitly** → UA's `contradicts` edge is a *materialized* version of
   what Cortex does today only at consolidation time. With it, contradiction is visible at
   **retrieval** time: the pre-thinking bundle can warn "doc X contradicts accepted ADR Y" before
   the model acts.
3. **Staleness signal** → maps onto `KnowledgeMeta.confidence` + the incremental
   fingerprint ([04-incremental-patching.md](04-incremental-patching.md)) re-scoring a claim when
   its source doc changes.
4. **Drill-down via MCP, not raw vectors** → a `claim`/`topic` graph is what makes
   `cortex_topic_drill(topic_id, dim="contradictions")` answerable as a graph walk.

In short: UA's knowledge mode is a concrete reference implementation of the second-brain contract
Cortex already decided it wants but hasn't fully graph-materialized.

---

## 6. What to adopt for Cortex's doc corpus

Cortex's documentation surface = `.rulebook/specs/`, `.rulebook/decisions/` (ADRs),
`docs/analysis/`, `CLAUDE.md`/`AGENTS.md`, plus any cross-repo docs. Plan:

| Step | Borrow | Cortex target |
|------|--------|---------------|
| Doc scanner + format detect | `knowledge-scanner`/`format-detector` | reuse Cortex's existing doc ingestion; add markdown wikilink/frontmatter parsing |
| `article`/`entity` extraction | deterministic markdown parser (F-6, `markdown-parser.ts`) | adapter parser → `document`/`article` + `entity` nodes |
| `claim` extraction + implicit edges | `article-analyzer` contract (§4) | LLM annotation worker, gated, evidence in `description` |
| `contradicts` / `builds_on` | `relationship-builder` | **fuse with Cortex's ADR `SUPERSEDES` chain** — a doc that cites a superseded ADR while contradicting the current one is the canonical alert |
| `topic` clustering | `relationship-builder` | unify with existing topic cards (don't create a parallel topic system) |
| confidence + staleness | `KnowledgeMeta.confidence` + fingerprint | re-score claims when source doc commits |

**Highest-value single feature:** wire `contradicts` edges between **docs and decisions**. That
turns "is our documentation consistent with our accepted decisions?" into a graph query, and lets
pre-thinking surface drift proactively — the exact gap `phase11r` flagged.

---

## 7. Caveats / non-goals

- **Don't build a parallel topic system.** Cortex already has topic cards + consolidation; the doc
  graph must *feed* them, not duplicate them.
- **Conservatism is load-bearing.** A `contradicts` edge with weak evidence is worse than none —
  keep UA's "explicit textual evidence only" rule and store the evidence span.
- **Format breadth is optional.** Cortex's docs are plain markdown + frontmatter; the full
  Obsidian/Logseq/Dendron detection matrix is over-scope. Support markdown + `[[wikilinks]]` +
  YAML frontmatter; skip the rest until a real KB needs it.
