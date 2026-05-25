# Timeline & Branching for Cortex — Executive Summary

> **Analysis ID:** TLB-001
> **Date:** 2026-05-24
> **Scope:** Add a temporal + branching dimension to Cortex so every fact (decision, snippet, code reference, learning, ADR, commit, incident) is grounded in time and in an explicit reasoning branch; supports point-in-time queries, cross-project correlation, and supersession reasoning.
> **Status:** Draft — pending review before promotion to Rulebook tasks.
> **Related analysis:** [code-doc-correlation](../code-doc-correlation/) (CDC-001).

---

## 1. The problem in one paragraph

Cortex holds knowledge from many HiveLLM projects (Cortex, Nexus, Vectorizer, Synap, Lexum, Rulebook, …) but treats facts as **flat and current**. A fix from last week and an ADR from a year ago sit in the same semantic space; a query for "how does X work?" can surface an obsolete decision because no temporal axis demotes it. There is also no concept of a **branch** — i.e., an exploration that forked off, was tried, and was either merged back or abandoned. The literature calls this failure mode "RAG is blind to time" — and it is exactly what the user has been experiencing.

## 2. The fix in one paragraph

Adopt **bitemporal modeling** (valid time + transaction time) plus an explicit **branch dimension** on every Cortex node and edge. Wire it into retrieval as a temporal classifier (`EXPIRED | VALID | TEMPORAL | SUPERSEDED`) that runs before ranking and demotes or removes stale facts. Add a project axis so cross-project relations (Cortex 0.x depends on Nexus 2.1) become first-class. Expose timeline and branch queries at the API/CLI/MCP layers so agents and humans can ask "what was true in project P at time T on branch B?" and get a grounded answer.

## 3. Why now

- The CDC-001 analysis (code↔doc correlation) identified supersession-aware ranking as a Tier-A fix. Timeline + branching is the **systemic** version of that fix: instead of patching ADR retrieval, give every entity a temporal/branch lifecycle.
- The user is actively reporting low retrieval relevance. Temporal staleness is a known root cause documented in the literature (Zep, T-GRAG, "RAG Is Blind to Time").
- HiveLLM has 7+ projects evolving in parallel; without a project + time axis, cross-project queries are unreliable.

## 4. Three concepts to internalize

| Concept | Plain meaning | Example in Cortex |
|---|---|---|
| **Bitemporality** | Two clocks per fact: when it was true in the world (valid time) and when Cortex recorded it (transaction time). | ADR-016 "accepted on 2026-03-12" (valid) but ingested on 2026-05-20 (transaction). |
| **Branch** | A parallel reasoning path; may merge or stay dormant. | Spec-11 fusion approach (main) vs. an earlier RRF-only attempt (abandoned branch). Both are knowledge worth keeping. |
| **Project axis** | Every fact is tagged with the project it belongs to and optionally with cross-project references. | "Cortex uses `nexus-graph-sdk = 2.1`" is a fact in both `cortex` and `nexus` timelines. |

## 5. What changes for the user

- `cortex timeline <project> --as-of <date>` — see the state of any project at any point in time.
- `cortex query "X" --as-of 2026-04-01 --branch main` — retrieval pinned to a moment and a branch.
- ADRs that were superseded last quarter stop polluting top-K. Decisions show their full supersession chain ordered by lifecycle and recency.
- Cross-project answers (e.g., "which projects depend on Nexus 2.1's external IDs?") become tractable via the project axis.
- Branch view shows abandoned approaches with reasons, so future agents do not re-try discarded paths.

## 6. Document map

- **[findings.md](findings.md)** — Literature survey: bitemporal modeling, temporal KGs (Zep, EvoKG, T-GRAG, Know-Evolve), "RAG is blind to time", and how each maps to Cortex.
- **[design.md](design.md)** — Concrete schema, edge taxonomy, query semantics, and integration points with Nexus/Meili/Vectorizer/pre-thinking.
- **[execution-plan.md](execution-plan.md)** — Phased rollout with deliverables, acceptance criteria, dependencies on CDC-001.
- **[references.md](references.md)** — Annotated bibliography.

## 7. Recommendation in one line

Land CDC-001 Phase 1 (eval harness) and Phase 2 fixes first, **then** start TLB-001 Phase 1 — the harness from CDC is what proves TLB's temporal classifier actually helps.
