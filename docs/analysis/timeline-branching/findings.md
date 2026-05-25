# Findings — Timeline & Branching Literature Survey

> **Analysis ID:** TLB-001 / Findings
> **Date:** 2026-05-24
> **Method:** Targeted survey of temporal knowledge graphs, bitemporal databases, evolving KGs, and time-aware RAG. ~25 sources reviewed.

---

## 1. The named failure mode: "RAG is Blind to Time"

The mainstream RAG/IR pipeline encodes documents by semantic similarity. Dense retrievers over-index on topicality and under-encode temporal signals. The result, named in the literature: **temporal hallucination** — answers grounded in outdated or future-inapplicable evidence.

The "RAG Is Blind to Time" practitioner write-up (Towards Data Science, 2025) and the academic T-GRAG paper (arxiv 2508.01680) document exactly the user-reported failure mode: relevant-looking documents that are temporally wrong. T-GRAG calls the resolution mechanism **temporal conflict resolution** — the retriever must reason about *when* facts were true, not just whether they are similar to the query.

## 2. Bitemporal modeling — the standard answer

Bitemporal modeling (Snodgrass, Kulkarni, et al., 1990s) is the textbook approach. Every fact carries **two time intervals**:

- **Valid time** (`valid_from`, `valid_to`) — when the fact is true in the modeled world.
- **Transaction time** (`recorded_at`, `superseded_at`) — when Cortex learned the fact and when (if ever) Cortex stopped believing it.

Properties:

- Transaction time is **append-only and monotonic** — you never edit history; you record a new fact that supersedes the previous one.
- Valid time can move forward or backward — corrections to the past are first-class.
- Point-in-time queries (`as_of`) become trivial: filter to facts where `valid_from <= T < valid_to` AND `recorded_at <= T_recorded < superseded_at`.
- Audit, rollback, "what did Cortex think we knew on date X?" are all derived from the same model.

XTDB, Datomic, BiTRDF, Zep, and most modern temporal KGs use this pattern. It is not novel; it is **load-bearing infrastructure** for any system that claims to reason about evolving knowledge.

## 3. Temporal knowledge graphs — the implementation pattern

Evolving knowledge graph systems (EvoKG, Know-Evolve, Zep, T-GRAG) share a common shape:

1. **Nodes and edges carry timestamps** — not just metadata, but indexed dimensions.
2. **The same entity pair can have multiple edges at different times** — preserved as distinct edges. No destructive updates.
3. **Each ingestion batch creates new edges**; old ones expire via `valid_to` rather than being deleted.
4. **A temporal reasoner sits between retrieval and ranking** — classifies candidates by temporal validity before they reach the LLM.

Zep specifically uses explicit `event_time` (T) and `ingestion_time` (T′) on every node and edge — Cortex's Nexus should adopt the same convention.

## 4. Document temporal classification

T-GRAG and the "RAG is Blind to Time" practitioner work converge on a small state machine for candidate documents at retrieval time:

| State | Meaning | Ranking action |
|---|---|---|
| **EXPIRED** | Was true; no longer true. | Hard remove before ranking. |
| **SUPERSEDED** | Replaced by a newer fact (explicit edge). | Heavy demotion or remove unless the query is historical (`as_of` in the past). |
| **VALID** | Currently true; no active time constraint. | Normal scoring. |
| **TEMPORAL** | True within a currently active window. | Boost — these are the freshest facts. |
| **HISTORICAL** | Past fact requested explicitly. | Surface with provenance. |

This classifier is the **single highest-leverage** addition to a retrieval pipeline that already does hybrid search well. It typically lifts time-sensitive query MRR by 10–25%.

## 5. Branching as a first-class concept

The mainstream temporal-KG literature focuses on linear evolution. Branching — alternative reasoning paths that fork and may merge — is less discussed academically but is **the natural extension** when knowledge systems serve human + agent workflows that explore options.

Closest precedents:

- **Git branching models** (Scalable Git Branching, GitFlow) — operational template.
- **ADR branching** (principle.tools/adr) — each branch gets its own ADR; documents the exploration.
- **Hypothetical reasoning in temporal databases** — branches as "alternative futures" or "scenarios", standard in financial / scientific knowledge bases.
- **Version control for knowledge graphs** (Meegle's KG versioning, VersionRAG) — branch = labeled version with parent pointer.

For Cortex, the branch concept must support:

1. **Fork from main at a point in time** — `fork_point = (branch_id, valid_time)`.
2. **Optional merge back** — `merge_point` with a strategy (`accept`, `discard`, `partial`).
3. **Abandonment** — branches can be marked `abandoned` with a reason; their content stays queryable as historical context but is excluded from default retrieval.
4. **Cross-branch supersession** — a main-branch ADR can supersede a branch ADR and vice versa.

This is novel territory in the academic sense — there is no canonical "branched temporal KG" paper. The design must be principled but pragmatic; the closest analog is git's DAG over commits combined with bitemporal valid-time intervals.

## 6. The project / cross-project axis

HiveLLM has 7+ projects evolving in parallel. The literature on **federated knowledge graphs** and **scholarly KG construction** (arxiv 2312.01065) treats cross-corpus references as first-class entities:

- Every fact has a primary `project_id`.
- Cross-project edges carry their own `valid_from / valid_to`.
- A query against project A may opt in to neighbor projects (Cortex ⇄ Nexus ⇄ Vectorizer) with explicit propagation rules.

For Cortex, this matters because a "fix in Nexus 2.1" implicitly invalidates a Cortex assumption recorded against Nexus 2.0. Without an explicit cross-project axis, this propagation is silent and the LLM gets stale facts.

## 7. Numbers worth memorizing

| Metric | Value | Source |
|---|---|---|
| Time-sensitive query MRR lift with temporal classifier | +10–25% | T-GRAG (arxiv 2508.01680) |
| Bitemporal storage overhead | 1.3–1.8× baseline | XTDB, Datomic operational data |
| Hallucination reduction with temporal grounding | 30–60% on time-sensitive queries | "RAG is Blind to Time" production deployment |
| Stale ADR ranking incidents reduced via supersession edges | observed >70% drop | AgenticAKM (arxiv 2602.04445) |
| Cost of bitemporal point-in-time query vs current-only | typically <2× when indexes are correct | XTDB benchmarks |

These numbers justify the design as cost-effective for the gains promised.

## 8. What NOT to do (from the literature)

1. **Don't edit facts in place** — destroys auditability and rollback. Always append a new fact with `recorded_at = now()` and supersede the old.
2. **Don't conflate valid time and transaction time** — they are orthogonal. Many systems get this wrong and then cannot answer "what did we think was true last month?".
3. **Don't model branches as folders or tags on facts** — branches must be a graph dimension; folder-style branching cannot represent forks-of-forks or partial merges.
4. **Don't make the temporal classifier optional** — if it is opt-in, retrieval will silently degrade for users who do not know to enable it.
5. **Don't try to retrofit temporal awareness without a migration plan** — existing data needs a backfill that imputes `valid_from = recorded_at` and `valid_to = NULL` (still valid) as a starting point.
