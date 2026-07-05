# Separate pipeline-hygiene gains from embedding-model ceiling when benchmarking retrieval quality

**Category**: retrieval
**Tags**: cortex, retrieval, embeddings, analysis:cortex-platform-2026-07

## Description

Cortex's embedding model (nomic-embed-text, 384-dim) caps raw cosine similarity at ~0.42-0.45 for a benchmark query regardless of pipeline hygiene: a ~90% corpus dedup/purge pass (phase26e) measurably shrank the corpus but did not raise the top-1 score past this ceiling. Lesson: when a quality metric plateaus despite real data-hygiene improvements, check whether a fixed upstream component (embedding model, reranker, tokenizer) imposes a ceiling independent of pipeline quality — only swapping that component raises the ceiling itself; pipeline hygiene only helps you reach it faster/more reliably.

## When to Use

When benchmarking retrieval/ranking quality improvements and a metric stops responding to further pipeline/data changes.
