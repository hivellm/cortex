# HiveGPU — Integrations & Relationships

## Hive-Vectorizer Integration

HiveGPU is designed as **optional GPU acceleration** for Hive-Vectorizer embedding pipelines.

**Current status**: Documented integration pattern; no hard dependency.

**Usage pattern**:
```rust
// Vectorizer generates embeddings (e.g., 384D or 768D)
let embeddings = vectorizer.encode_batch(&texts).await?;

// HiveGPU stores & searches
let context = CudaContext::new()?;
let mut storage = context.create_storage(384, GpuDistanceMetric::Cosine)?;
storage.add_vectors(&vectors)?;
let results = storage.search(&query, 10)?;
```

**Integration points**:
- Vectorizer outputs f32 slices → HiveGPU consumes via `GpuVector`
- No SDK changes to Vectorizer required
- Orthogonal: Vectorizer continues to run on CPU; HiveGPU handles search acceleration

## Cortex (This Project)

HiveGPU is a **consolidation target** for Cortex's knowledge base:
- Cortex classifies/indexes documents via embeddings
- HiveGPU can accelerate similarity search at scale

**Planned integration**:
- Cortex classifier worker produces embeddings → push to HiveGPU storage
- Search queries route to GPU for fast retrieval
- No refactoring of Cortex internals required

## Expert (Inference Service)

**No direct integration** in 0.2.x. Expert runs inference on CPU/GPU; HiveGPU does not replace Expert's compute.

**Future relevance** (v0.4):
- GPU HNSW construction might leverage Expert's GPU if both run on same host
- Quantization (PQ/SQ) could integrate Expert's learned codebooks

## Nexus (Graph Database)

**No direct integration**. Nexus stores structured knowledge; HiveGPU is read-only vector search.

**Potential confluence** (future):
- Cortex embeds Nexus nodes → HiveGPU indexes embeddings → Cortex links search results back to Nexus graph

## Synap (Streaming/Telemetry)

**No integration** in 0.2.x.

**Operational value** (future):
- HiveGPU metrics (search latency, VRAM usage, throughput) → Synap observability pipeline

## Lexum (Semantic Search)

**Orthogonal to HiveGPU**. Lexum does semantic indexing on CPU; HiveGPU is GPU acceleration for the same workload.

**Competitive/complementary**:
- Lexum: CPU-only, broad language support, large ecosystem
- HiveGPU: GPU-only, Rust-native, extreme performance for production workloads

## No Monorepo Dependencies

HiveGPU is **standalone** and does not require:
- Cortex crates
- Expert SDK
- Nexus SDK
- Any HiveLLM infrastructure

It consumes:
- Standard Rust ecosystem (cudarc, objc2, ash, etc.)
- Metal/CUDA/ROCm/Vulkan driver APIs (runtime, not build-time)

## Versioning & Compatibility

- **HiveGPU v0.2.0**: Compatible with Rust 1.70+, cudarc 0.13, objc2-metal 0.3.x, naga 23
- **Breaking changes**: None planned until v0.3 (HNSW addition)
- **Upstreaming**: All backends pull from standard Cargo crates (no vendoring)

## Distribution & Consumption

HiveGPU is published on:
- [crates.io](https://crates.io/crates/hive-gpu)
- [docs.rs](https://docs.rs/hive-gpu)
- GitHub: [hivellm/hive-gpu](https://github.com/hivellm/hive-gpu)

Consumers add to `Cargo.toml`:
```toml
hive-gpu = "0.2.0"                                   # Metal (macOS default)
hive-gpu = { version = "0.2.0", features = ["cuda"] } # CUDA (Linux/Windows)
```
