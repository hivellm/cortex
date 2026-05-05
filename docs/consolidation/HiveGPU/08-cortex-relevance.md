# HiveGPU — Cortex Relevance & Ingestion Priorities

## Why HiveGPU Matters to Cortex

Cortex's core job is capturing, indexing, and retrieving knowledge from 17 HiveLLM projects. Currently:

- **Indexing**: Cortex embeds documents via Vectorizer → stores in Meilisearch (CPU-only full-text index)
- **Bottleneck**: Semantic similarity search (the hard problem) is done at application layer, not at indexing scale

HiveGPU **accelerates** the semantic search half of that pipeline:

1. Cortex classifier produces embeddings (384D or 768D f32 vectors)
2. HiveGPU stores them on GPU (VRAM-resident, zero-copy search)
3. Queries execute 3–100× faster than CPU brute-force
4. Results feed back into Cortex's ranking/deduplication logic

## High-Priority Integration Points

### 1. Embedding Storage & Search (Priority: HIGH)

**Current state**: Embeddings live in memory or disk; search is CPU-bound.

**HiveGPU fit**: Store all embeddings in GPU; search via SGEMV (CUDA) or Metal compute kernels.

**Cortex change required**:
- Post-embed hook: `cortex_storage.py` → push `GpuVector` batch to HiveGPU storage
- Search: swap CPU dot-product loop for `storage.search(&query_embedding, 100)`
- Performance gain: 3–10× on 100K+ vectors (depending on dim, GPU)

**Effort**: ~2–3 days (trait adoption, plumbing)

### 2. Consolidation Indexing (Priority: MEDIUM)

If Cortex consolidates archives from 17 repos into a single index:

- **Bootstrap**: 1M+ embeddings across repos (typical)
- **Batch ingest**: 10K vectors/second on RTX 4090 (1.4M elem/s throughput)
- **Total time**: ~1 minute for 1M vectors (vs. 10+ minutes CPU scan)

**HiveGPU advantage**: Consolidation job runs fast; Cortex can re-index hourly instead of weekly.

### 3. IVF Index for Cross-Project Recall (Priority: MEDIUM)

Once embedding count grows beyond 10K:

- **Brute-force latency**: ~4 ms at 100K vectors (still fast; GPUs win)
- **IVF speedup**: 100–1000× faster at 1M vectors (3.67× on RTX 4090)
- **Recall trade-off**: ≥0.95 with proper `nprobe` tuning

**Cortex use**: Run IVF for "broad retrieval" (top-100 candidates) → rerank with dense classifier.

**Effort**: ~1 week (IVF parameter tuning, recall validation)

## Medium-Priority: Operational Concerns

### 4. Multi-GPU / Multi-Tenant Deployment

HiveGPU supports one context per GPU; if Cortex runs on multi-GPU host:

- Create separate storage per GPU
- Distribute vector batch across GPUs (manual in 0.2.x)
- Aggregate results

**Current status**: Not automated; would require wrapper layer.

**Timeline**: v0.3+ or Cortex layer on top.

### 5. Persistence / Checkpoint (Priority: LOW)

HiveGPU IVF index does not checkpoint in 0.2.x. If Cortex needs durable index:

- Option A: Cortex exports embeddings + centroids → serialize to BSON/Protobuf
- Option B: Wait for HiveGPU v0.4 snapshot support
- Option C: Re-cluster on startup (acceptable if <1 min; typical for <1M vectors)

**Recommendation**: Defer; start with in-memory IVF.

## Ingestion Strategy (Recommended Sequence)

### Phase 1: Baseline (Week 1)
- Add HiveGPU dependency to Cortex (feature-gated to avoid bloat on CPU hosts)
- Implement `cortex_embeddings.rs` module with `EmbeddingStore` trait
- Wire `CudaContext` initialization at Cortex startup
- Add device-info logging ("GPU: RTX 4090, 24GB VRAM")

### Phase 2: Brute-Force Search (Week 2)
- Swap embedding search loop: CPU dot-product → `storage.search()`
- Benchmark: capture latency profiles (100K / 1M vectors)
- Document performance gains in Cortex README

### Phase 3: IVF Indexing (Week 3)
- Implement `IvfSearcher` wrapper
- Add config knobs: `n_list`, `nprobe`, `kmeans_iterations`
- Validate recall on real Cortex data (expect ≥0.95)
- Tune for Cortex's typical query patterns

### Phase 4: Monitoring & Logging (Week 4)
- Add VRAM usage metrics → observability pipeline
- Log search latency percentiles
- Add fallback: if VRAM exhausted, degrade to CPU

## Success Criteria

1. **Performance**: End-to-end search latency ≤100 ms for 100K vectors (GPU + ranking)
2. **Recall**: ≥0.95 on IVF index (measured against brute-force ground truth)
3. **Reliability**: GPU OOM gracefully degrades to CPU (no crash)
4. **Observability**: Search latency p50/p95/p99 tracked and exposed

## Constraints & Gotchas

1. **Single backend per Cortex instance** (for now): No Metal + CUDA simultaneously
   - Rationale: Feature flags are compile-time; runtime selection needed later
   
2. **f32 vectors only**: Cortex embeddings must be f32 (compatible with HiveGPU)
   - Typical: 384D or 768D f32 = 1.5–3 MB per vector (acceptable)
   
3. **VRAM capacity**: 8 GB GPU → ~2.6M vectors (128-dim) or 680K (768-dim)
   - Rule of thumb: 3–4 bytes per vector dimension
   - Plan for 50–80% capacity (margin for IVF centroids + intermediate allocs)
   
4. **No persistence yet**: IVF index is ephemeral; rebuild on restart
   - Acceptable for <100K vectors (< 10 sec rebuild)
   - Plan for checkpoint after v0.4

## Related Cortex Tasks

- **cortex-embedder**: GPU acceleration already done here (separate crate); no duplication
- **cortex-classifier-worker**: Produces embeddings; will push to HiveGPU storage
- **cortex-consolidator**: Would benefit from GPU batch ingest
- **cortex-api**: Add `GET /search/gpu` endpoint (optional, future)

## Knowledge Base Maintenance

Update Cortex's consolidation KB when:
- HiveGPU 0.3 ships (HNSW, quantization changes)
- Cortex implements phase 1 + 2 (baseline + brute-force)
- Hardware validation completes for Metal/ROCm/Intel (phase4d/e/f)
