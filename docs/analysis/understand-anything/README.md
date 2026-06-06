# Understand-Anything — Analysis for Cortex

> **Analysis ID:** UA-001
> **Date:** 2026-06-06
> **Source:** https://github.com/Lum1104/Understand-Anything
> **Scope:** What Cortex can adopt from Understand-Anything's codebase→knowledge-graph pipeline
> **Status:** Complete

---

## 1. Executive Summary

**Understand-Anything (UA)** is a cross-platform AI-assistant plugin (Claude Code, Cursor,
Copilot, Codex, Gemini CLI) that turns any codebase / knowledge base / docs into an
**interactive knowledge graph** you can search, explore, and tour. TypeScript pnpm monorepo,
LLM + tree-sitter hybrid, 9 specialized agents, fingerprint-based incremental updates.

**Why it matters to Cortex:** UA solves the *exact* sub-problem Cortex needs for its code/doc
graph lane (`IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES` relations referenced in the pre-thinking
pipeline) — deterministic structural extraction fused with LLM semantics, serialized to a
versioned graph, kept fresh by git-diff incremental patching. UA is **architecturally aligned**
with Cortex's "deterministic facts + LLM semantics + RRF retrieval" thesis, but operates at the
*repo-structure* altitude where Cortex is currently thinner.

**Headline takeaways (ranked):**

| # | Takeaway | Cortex target | Value |
|---|----------|---------------|-------|
| 1 | **Fingerprint-based incremental graph patching** (git-hash staleness + change classifier + surgical node/edge merge) | `cortex-workers` graph indexer | High — avoids full re-embed/re-graph on every commit |
| 2 | **35-type edge taxonomy + 21-type node taxonomy** spanning code, infra, data, domain, knowledge | Nexus graph schema | High — ready-made ontology superset of Cortex's current relations |
| 3 | **Two-phase file-analyzer** (tree-sitter deterministic → LLM semantic) with strict "never invent paths / import count must match" guards | adapter/worker extraction | High — anti-hallucination contract is directly portable |
| 4 | **Change classifier** (SKIP / PARTIAL / ARCHITECTURE / FULL) gating how much downstream re-analysis runs | consolidation scheduler | Medium — tiered re-work budget |
| 5 | **Karpathy-pattern knowledge-base parsing** (wikilinks + implicit-relationship + claim/entity/topic/source nodes) | docs/wiki lane | Medium — Cortex already cites Karpathy; UA has a concrete graph for it |
| 6 | **12 specialized non-code parsers** (Dockerfile, SQL, GraphQL, Terraform, protobuf, TOML…) | adapter coverage | Medium — extends graph beyond source code |
| 7 | **Auto-update via PostToolUse(git commit)+SessionStart hooks** | Cortex hook surface | Low/Medium — pattern already partially present in Cortex |

**Bottom line:** adopt the **node/edge ontology** and the **incremental-patching algorithm**
first; they are the highest-leverage, lowest-risk borrows. Treat UA's local Fuse.js + in-memory
cosine search as a *non-target* — Cortex already has a stronger hybrid (vector+keyword+graph RRF).

See [findings.md](02-findings.md) for the numbered, evidenced findings and
[adoption.md](07-adoption.md) for the concrete mapping to Cortex crates + a phased plan.

### Documents in this analysis

| # | File | Contents |
|---|------|----------|
| — | [README.md](README.md) | This file — executive summary, architecture, non-targets |
| 01 | [01-file-inventory.md](01-file-inventory.md) | Every repo file enumerated + per-file Cortex relevance + reading order |
| 02 | [02-findings.md](02-findings.md) | F-1…F-9 evidenced findings |
| 03 | [03-ontology-mapping.md](03-ontology-mapping.md) | UA↔Cortex/Nexus node+edge crosswalk |
| 04 | [04-incremental-patching.md](04-incremental-patching.md) | Git-hash staleness + change classifier + merge → Rust spec |
| 05 | [05-extraction-contract.md](05-extraction-contract.md) | Deterministic-gated LLM annotation + reconciliation gate |
| 06 | [06-parsers.md](06-parsers.md) | 12 non-code parsers + framework registry coverage map |
| 07 | [07-adoption.md](07-adoption.md) | Borrow/adapt/reject matrix + phased plan + open questions |
| 08 | [08-knowledge-docs.md](08-knowledge-docs.md) | **Docs/wiki as a graph** — claims, contradictions, topic clustering for Cortex's doc+ADR corpus |

---

## 2. What UA Is (architecture in one screen)

```
/understand  ─▶ project-scanner ─▶ file-analyzer (×N, ≤5 concurrent, 20–30 files/batch)
                                        │  Phase 1: extract-structure.mjs (tree-sitter, 10 langs
                                        │           + 12 non-code parsers) → deterministic facts
                                        │  Phase 2: LLM → GraphNode[] / GraphEdge[] (summaries,
                                        │           complexity, semantic edges)
                                        ▼
              architecture-analyzer ─▶ layers
              domain-analyzer       ─▶ domain/flow/step nodes (business processes)
              tour-builder          ─▶ dependency-ordered guided tour (persona-aware)
              graph-reviewer        ─▶ referential-integrity validation
                                        ▼
                         .understand-anything/knowledge-graph.json   (versioned, git-lfs ready)
                                        ▼
   dashboard: pan/zoom, Fuse.js fuzzy + cosine semantic search, diff-impact, domain view
```

**Incremental path:** `staleness.isStale()` compares stored `gitCommitHash` vs `HEAD` →
`git diff <hash>..HEAD --name-only` → `change-classifier.classifyUpdate()` picks
SKIP/PARTIAL/ARCHITECTURE/FULL → `mergeGraphUpdate()` removes nodes for changed `filePath`s,
prunes dangling edges, merges freshly analyzed nodes/edges, stamps new hash+timestamp.

---

## 3. Tech stack

- TypeScript 70% / JS 16% / Python 10% / Astro 3%; pnpm workspace monorepo; Vitest; ESLint.
- Core lib: `understand-anything-plugin/packages/core/src/` — `types.ts`, `schema.ts`,
  `search.ts` (Fuse.js), `embedding-search.ts` (in-memory cosine), `staleness.ts`,
  `change-classifier.ts`, `plugins/parsers/*`.
- Agents: 9 markdown agent defs under `understand-anything-plugin/agents/`.
- Hooks: `hooks/hooks.json` (PostToolUse + SessionStart) + `auto-update-prompt.md`.

---

## 4. Non-targets / what NOT to copy

- **Fuse.js bitap fuzzy search** + **linear in-memory cosine** — fine for a single-repo dashboard;
  Cortex's HNSW + BM25 + graph RRF is strictly more capable. Don't regress.
- **Single JSON file graph** (`knowledge-graph.json`) — UA's transport; Cortex persists to Nexus.
  Borrow the *schema*, not the *storage*.
- **TypeScript impl** — Cortex is Rust; port concepts, not code.

---

## 5. License / attribution check

Verify the UA license before lifting any code verbatim (concepts/ontology are not copyrightable,
but agent-prompt text and parser code are). Default stance: **reimplement from the spec in
[adoption.md](07-adoption.md)**, cite UA as prior art in the ADR.
