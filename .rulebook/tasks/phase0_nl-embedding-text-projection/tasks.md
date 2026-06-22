## 1. Natural-language embedding projection
- [x] 1.1 Add a deterministic NL projection of each event payload (descriptive text) as the embedder's input text, replacing raw-JSON embedding; works with the classifier in Static mode
- [x] 1.2 Unit-test the projection (raw payload → readable NL text) for the main kinds (turn, tool_call, decision, artifact)

## 2. Re-index + measure
- [x] 2.1 Re-embed / re-index the corpus through the new projection (Vectorizer dense lane)
- [x] 2.2 Re-run `cortex-eval --suite retrieval` fusion-only AND reranked; confirm pre-rerank vector MRR rises and recall@5 returns to 1.0; record deltas in `crates/cortex-eval/baselines/cdc-baseline-v1.json`

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation
- [x] 3.2 Write tests covering the new behavior
- [x] 3.3 Run tests and confirm they pass
