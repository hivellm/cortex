## 1. Candidate shortlist and research
- [ ] 1.1 Shortlist 2-3 candidate embedding models compatible with the Vectorizer SDK's supported backends
- [ ] 1.2 For each candidate, record dimensionality, code-aware training (yes/no), licensing, and self-hosting vs hosted-API latency/cost profile
- [ ] 1.3 Prioritize candidates with higher dimensionality or code-aware training, given the corpus's large source-code-chunk share

## 2. Benchmark against the real golden set
- [ ] ⏸ 2.1 blocked on `phase28_retrieval-eval-gate-live`: run each shortlisted candidate against the real (non-placeholder) golden set once it exists
- [ ] ⏸ 2.2 blocked on `phase28_retrieval-eval-gate-live`: measure recall@5, MRR@10, p50/p95 embedding latency, and per-embedding cost (hosted) or resource cost (self-hosted) per candidate

## 3. Ceiling re-test (like-for-like)
- [ ] 3.1 Re-run the exact benchmark query used in the 2026-07-05 analysis and phase26e ("event classification system", repo `cortex`, per-source score via `/v1/query`) against each candidate
- [ ] 3.2 Confirm whether each candidate raises the raw-cosine ceiling (~0.42-0.45) itself, not just whether the fused/reranked score changes

## 4. Recommendation and ADR
- [ ] 4.1 Produce a stay/migrate recommendation backed by the measured recall/MRR/latency/cost deltas from §2-§3
- [ ] 4.2 If recommending migration: draft a concrete migration plan (full corpus re-embed, dual-write/cutover strategy, rollback plan)
- [ ] 4.3 Record the recommendation as an ADR

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
