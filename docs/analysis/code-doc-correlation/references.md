# References — Code↔Documentation Correlation Survey

> **Analysis ID:** CDC-001 / References
> **Date:** 2026-05-24
> **Method:** Web search across academic literature (arXiv, ACL, MDPI, ResearchGate) + industry technical sources. 28 sources reviewed.

---

## Primary academic papers

### Traceability link recovery — classical IR era

1. **Antoniol et al., "Recovering Traceability Links between Code and Documentation"** — Foundational IR-based approach with LSI. [PDF](https://hiper.cis.udel.edu/lp/lib/exe/fetch.php/courses/other-traceability-antonioltse.pdf)
2. **Information Retrieval based requirement traceability recovery — systematic literature review** — Survey of VSM/LSI/LDA methods. [Academia.edu](https://www.academia.edu/37684404/Information_Retrieval_based_requirement_traceability_recovery_approaches_A_systematic_literature_review)
3. **"Recovery of traceability links between software documentation and source code"** — Early IR experiments. [Academia.edu](https://www.academia.edu/2722350/Recovery_of_traceability_links_between_software_documentation_and_source_code)
4. **"A Literature Review of Automatic Traceability Links Recovery" (ICPC 2020)** — Sui et al. survey covering VSM through deep learning. [PDF](https://yuleisui.github.io/publications/icpc20.pdf)
5. **"Advancing Trace Recovery Evaluation"** — Applied IR in software engineering. [arXiv 1602.07633](https://arxiv.org/pdf/1602.07633)
6. **"How to Effectively Use Topic Models for Software Engineering Tasks?" (ICSE 2013)** — LDA tuning study. [PDF](https://www.cs.wm.edu/~denys/pubs/ICSE'13-LDA-CRC.pdf)

### Traceability link recovery — deep learning era

7. **"Semantically Enhanced Software Traceability Using Deep Learning"** — Early DL approach. [arXiv 1804.02438](https://arxiv.org/pdf/1804.02438)
8. **"Enhancing Traceability Link Recovery with Fine-Grained Query Expansion Analysis" (MDPI 2023)** — Query expansion for IR-based trace recovery. [MDPI](https://www.mdpi.com/2078-2489/14/5/270)
9. **"Improving the Effectiveness of Traceability Link Recovery using Hierarchical Bayesian Networks"** — [arXiv 2005.09046](https://arxiv.org/pdf/2005.09046)
10. **"Enhancing Automated Software Traceability by Transfer Learning from Open-World Data"** — [ResearchGate](https://www.researchgate.net/publication/361763839_Enhancing_Automated_Software_Traceability_by_Transfer_Learning_from_Open-World_Data)
11. **"TRIAD: Automated Traceability Recovery based on Biterm-enhanced Deduction of Transitive Links"** — [arXiv 2312.16854](https://arxiv.org/pdf/2312.16854)
12. **"Enhancing Requirements Traceability Link Recovery: A Novel Approach with T-SimCSE"** — [arXiv 2603.11800](https://arxiv.org/html/2603.11800)

### Traceability — LLM era

13. **"Evaluating the Use of LLMs for Documentation to Code Traceability"** — Key numbers: Claude 3.5 Sonnet F1 0.794 vs BM25 0.441. [arXiv 2506.16440](https://arxiv.org/html/2506.16440v1) · [PDF](https://www.arxiv.org/pdf/2506.16440)

### Code embedding models

14. **CodeBERT** — Pre-trained model for programming + natural languages. [arXiv 2002.08155](https://arxiv.org/pdf/2002.08155)
15. **GraphCodeBERT** — Pre-training with data flow. [arXiv 2009.08366](https://arxiv.org/pdf/2009.08366)
16. **UniXcoder** — Unified cross-modal pre-training for code. [ACL Anthology 2022](https://aclanthology.org/2022.acl-long.499.pdf)
17. **CoCoSoDa** — Contrastive learning for code search. +13.3% MRR over CodeBERT. [arXiv 2204.03293](https://arxiv.org/pdf/2204.03293)
18. **CodeCSE** — Multilingual code+comment embeddings. [arXiv 2407.06360](https://arxiv.org/html/2407.06360v1)
19. **BatCoder** — Self-supervised bidirectional code-doc back-translation. [arXiv 2602.02554](https://arxiv.org/pdf/2602.02554)
20. **AI-Assisted Programming Tasks Using Code Embeddings and Transformers** — [MDPI Electronics](https://www.mdpi.com/2079-9292/13/4/767)
21. **"Fine-Grained Features-based Code Search for Precise Retrieval"** — [COLING 2025](https://aclanthology.org/2025.coling-main.482.pdf)

### Cross-modal alignment

22. **"Mind the Gap: A Generalized Approach for Cross-Modal Embedding Alignment"** — [arXiv 2410.23437](https://arxiv.org/pdf/2410.23437)
23. **"Enhancing Cross-Language Code Translation via Task-Specific Embedding Alignment in RAG"** — [arXiv 2412.05159](https://arxiv.org/pdf/2412.05159)

### Repository-level RAG and chunking

24. **"Retrieval-Augmented Code Generation: A Survey with Focus on Repository-Level Approaches"** — [arXiv 2510.04905](https://arxiv.org/pdf/2510.04905)
25. **"RepoQA: Evaluating Long Context Code Understanding"** — [arXiv 2406.06025](https://arxiv.org/pdf/2406.06025)
26. **"How Does Chunking Affect Retrieval-Augmented Code Completion? A Controlled Empirical Study"** — [arXiv 2605.04763](https://arxiv.org/html/2605.04763v1)
27. **"Relative Positioning Based Code Chunking Method for Rich Context Retrieval"** — [arXiv 2510.08610](https://arxiv.org/html/2510.08610v1)
28. **"Beyond More Context: How Granularity and Order Drive Code Completion Quality"** — [arXiv 2510.06606](https://arxiv.org/pdf/2510.06606)
29. **"aiXcoder-7B-v2: Training LLMs to Fully Utilize the Long Context in Repository-level Code Completion"** — [arXiv 2503.15301](https://arxiv.org/pdf/2503.15301)
30. **CodeRAG-Bench** — First large-scale code retrieval and RAG benchmark. [Project page](https://code-rag-bench.github.io/)

### RAG surveys and architectures

31. **"A Systematic Review of Key Retrieval-Augmented Generation (RAG) Systems"** — [arXiv 2507.18910](https://arxiv.org/html/2507.18910v1)
32. **"Retrieval-Augmented Generation: A Comprehensive Survey of Architectures, Enhancements, and Robustness Frontiers"** — [arXiv 2506.00054](https://arxiv.org/html/2506.00054v1)

### Hallucination measurement and reduction

33. **"Measuring and Reducing LLM Hallucination without Gold-Standard Answers" (FEWL)** — [arXiv 2402.10412](https://arxiv.org/pdf/2402.10412)
34. **"How Much Do LLMs Hallucinate in Document Q&A Scenarios? A 172-Billion-Token Study"** — [arXiv 2603.08274](https://arxiv.org/pdf/2603.08274)

### ADR generation and management with LLMs

35. **"AgenticAKM: Enroute to Agentic Architecture Knowledge Management"** — [arXiv 2602.04445](https://arxiv.org/html/2602.04445v1) · [PDF](https://arxiv.org/pdf/2602.04445)
36. **"Context Matters: Evaluating Context Strategies for Automated ADR Generation Using LLMs"** — [arXiv 2604.03826](https://arxiv.org/html/2604.03826)
37. **"Can LLMs Generate Architectural Design Decisions? — An Exploratory Empirical Study"** — [arXiv 2403.01709](https://arxiv.org/html/2403.01709v1)

### Code knowledge graphs

38. **"A Toolkit for Generating Code Knowledge Graphs"** — [arXiv 2002.09440](https://arxiv.org/pdf/2002.09440)
39. **"Scholarly Knowledge Graph Construction from Published Software Packages"** — [arXiv 2312.01065](https://arxiv.org/pdf/2312.01065)

---

## Industry technical references

40. **"Building Hybrid Search That Actually Works: BM25 + Dense Retrieval + Cross-Encoders"** — Ranjan Kumar. [Article](https://ranjankumar.in/building-a-full-stack-hybrid-search-system-bm25-vectors-cross-encoders-with-docker)
41. **"Hybrid Search: BM25 and Dense Retrieval Combined"** — Michael Brenndoerfer. [Article](https://mbrenndoerfer.com/writing/hybrid-search-bm25-dense-retrieval-fusion)
42. **"Hybrid RAG in the Real World: Graphs, BM25, and the End of Black-Box Retrieval"** — NetApp. [Article](https://community.netapp.com/t5/Tech-ONTAP-Blogs/Hybrid-RAG-in-the-Real-World-Graphs-BM25-and-the-End-of-Black-Box-Retrieval/ba-p/464834)
43. **"Hybrid Search in Production: Why BM25 Still Wins"** — TianPan. [Article](https://tianpan.co/blog/2026-04-12-hybrid-search-production-bm25-dense-embeddings)
44. **"Optimizing RAG with Hybrid Search & Reranking"** — Superlinked VectorHub. [Article](https://superlinked.com/vectorhub/articles/optimizing-rag-with-hybrid-search-reranking)
45. **"Best Practices for Mitigating Hallucinations in LLMs"** — Microsoft Azure AI Foundry. [Article](https://techcommunity.microsoft.com/blog/azure-ai-foundry-blog/best-practices-for-mitigating-hallucinations-in-large-language-models-llms/4403129)
46. **"What embedding models work best for code and technical content?"** — Zilliz. [Article](https://zilliz.com/ai-faq/what-embedding-models-work-best-for-code-and-technical-content)
47. **"Building a Knowledge Graph of Your Codebase"** — Daytona. [Article](https://www.daytona.io/dotfiles/building-a-knowledge-graph-of-your-codebase)
48. **"CodeGraph: Build Queryable Knowledge Graphs from Code"** — FalkorDB. [Article](https://www.falkordb.com/blog/code-graph/)

---

## Cross-references back into this analysis

- The seven consolidated principles in [findings.md](findings.md) are grounded in items 13 (LLM traceability), 30 + 24–28 (RAG/chunking), 40–44 (hybrid retrieval), 35–37 (ADR management), and 33–34 (hallucination).
- The gap mapping in [gaps.md](gaps.md) cites items 13, 22–23, 26–28, 30, 35, and 45 explicitly.
- The execution sequencing in [execution-plan.md](execution-plan.md) follows the "measure before optimize" pattern argued in items 30, 31, 33, and 34.

---

## How to add to this bibliography

When new evidence surfaces:
1. Append the entry with a one-line description and the URL.
2. Cross-reference it from the relevant section in `findings.md` or `gaps.md`.
3. Update the count at the top of this file.
4. If the entry changes a tier-A recommendation in `execution-plan.md`, surface it explicitly.
