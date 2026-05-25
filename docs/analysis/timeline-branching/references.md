# References — Timeline & Branching Literature Survey

> **Analysis ID:** TLB-001 / References
> **Date:** 2026-05-24

---

## Bitemporal modeling

1. **Snodgrass, R.T. — "Developing Time-Oriented Database Applications in SQL"** — Foundational bitemporal reference. Defines valid time vs transaction time.
2. **Bitemporal Modeling Overview** — EmergentMind topic page. [Article](https://www.emergentmind.com/topics/bitemporal-modeling)
3. **Wikipedia — Temporal database** — [Article](https://en.wikipedia.org/wiki/Temporal_database)
4. **XTDB — Bitemporality concept** — Production-grade bitemporal store. [Docs](https://v1-docs.xtdb.com/concepts/bitemporality/)
5. **JUXT — The Value of Bitemporality** — Industry rationale for bitemporal modeling. [Blog](https://www.juxt.pro/blog/value-of-bitemporality/)
6. **"Bitemporality, or how to change the past"** — Marley Spoon engineering. [DEV](https://dev.to/marleyspoon/bitemporality-or-how-to-change-the-past-3k4f)
7. **BiTemporal RDF Model (Math 2025)** — Adds valid + transaction time to RDF. [DOI](https://doi.org/10.3390/math13132109)
8. **"Specification and Implementation of Temporal Databases in a Bitemporal Event Calculus"** — Springer. [Chapter](https://link.springer.com/chapter/10.1007/3-540-48054-4_7)
9. **"A Temporal Study of Data Sources to Load a Corporate Data Warehouse"** — Springer. [Chapter](https://link.springer.com/chapter/10.1007/978-3-540-45228-7_12)

## Temporal knowledge graphs

10. **Know-Evolve: Deep Temporal Reasoning for Dynamic Knowledge Graphs (arXiv 1705.05742)** — [PDF](https://arxiv.org/pdf/1705.05742)
11. **Zep — Temporal Knowledge Graph Architecture** — Explicit event-time + ingestion-time per node/edge. [Topic](https://www.emergentmind.com/topics/zep-a-temporal-knowledge-graph-architecture)
12. **EvoKG: Evolving Knowledge Graph Systems** — [Topic](https://www.emergentmind.com/topics/evokg)
13. **Knowledge Graph Versioning — Meegle** — Versioning patterns including supersession. [Article](https://www.meegle.com/en_us/topics/knowledge-graphs/knowledge-graph-versioning)
14. **"A Temporal Knowledge Graph Built from Simple Signals"** — Operational example. [Medium](https://medium.com/devops-ai/a-temporal-knowledge-graph-built-from-simple-signals-f74df1bd8c84)
15. **"Self-Aware Vector Embeddings for RAG" (arXiv 2604.20598)** — Neuroscience-inspired temporal + confidence embeddings. [arXiv](https://arxiv.org/html/2604.20598v1)

## Time-aware retrieval / RAG temporal grounding

16. **"RAG Is Blind to Time — I Built a Temporal Layer to Fix It in Production"** — Production write-up; document classifier EXPIRED / VALID / TEMPORAL. [Article](https://towardsdatascience.com/rag-is-blind-to-time-i-built-a-temporal-layer-to-fix-it-in-production/)
17. **T-GRAG: Dynamic GraphRAG for Resolving Temporal Conflicts (arXiv 2508.01680)** — [PDF](https://arxiv.org/pdf/2508.01680)
18. **RAG Meets Temporal Graphs (arXiv 2510.13590)** — Time-sensitive modeling for evolving knowledge. [PDF](https://arxiv.org/pdf/2510.13590) · [HTML](https://arxiv.org/html/2510.13590v1)
19. **"Efficient Temporal-aware Matryoshka Adaptation for Temporal Information Retrieval" (arXiv 2601.05549)** — [PDF](https://arxiv.org/pdf/2601.05549)
20. **Temporal Retrieval-Augmented Generation (RAG) — overview** — [Topic](https://www.emergentmind.com/topics/temporal-retrieval-augmented-generation-rag)
21. **"Grounding LLMs with Fresh Web Data to Reduce Hallucinations"** — [Article](https://towardsdatascience.com/grounding-llms-with-fresh-web-data-to-reduce-hallucinations/)
22. **"What Is LLM Grounding? A Developer's Guide"** — [Article](https://neuledge.com/blog/2026-02-20/what-is-llm-grounding)

## Branching and versioning of knowledge

23. **Architecture Decision Record: Branching Strategies** — principle.tools template. [Article](https://principle.tools/adr/01-branching-strategies/)
24. **Claude ADR System Guide** — ADR system with branching integration. [Gist](https://gist.github.com/joshrotenberg/a3ffd160f161c98a61c739392e953764)

## Software timelines and visualization

25. **Gource — software version control visualization** — Visual analogy for project timelines. [Site](https://gource.io/)
26. **"Tools to visualize the history of a git repository"** — Survey. [Article](https://livablesoftware.com/tools-to-visualize-the-history-of-a-git-repository/)
27. **"Software Development Timelines: Plan, Predict & Deliver"** — Industry framing. [Article](https://neontri.com/blog/software-development-timelines/)

## ADR / architecture knowledge graphs (cross-link with CDC-001)

28. **AgenticAKM (arXiv 2602.04445)** — Agentic Architecture Knowledge Management; multi-artifact grounding. [arXiv](https://arxiv.org/html/2602.04445v1)
29. **Scholarly Knowledge Graph Construction from Published Software Packages (arXiv 2312.01065)** — Federated cross-project pattern. [PDF](https://arxiv.org/pdf/2312.01065)

## Cross-references

- The "temporal hallucination" / "RAG is blind to time" thesis (items 16, 17, 18) is the root justification for TLB-001 in `findings.md §1`.
- The bitemporal model (items 1–9) is the schema basis in `design.md §1.1`.
- The state-machine classifier in `design.md §2.2` is adapted from item 16 with extensions for SUPERSEDED and ABANDONED states.
- CDC-001's supersession Tier-A item (gaps.md Gap 4) is **superseded** by TLB-001's temporal classifier once Phase 2 lands.

---

## How to extend this bibliography

Add new entries under the appropriate section, then cross-link from `findings.md` or `design.md`. If a new reference changes a Tier-A recommendation in `execution-plan.md`, flag it explicitly in that file.
