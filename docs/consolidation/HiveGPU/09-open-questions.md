# HiveGPU — Open Questions & Undocumented Items

Gaps, unclear decisions, and deferred work identified during consolidation.

## Architecture & Design

- **Backend detection override mechanism (`HIVE_GPU_BACKEND` env var)**: Documented as "planned" in D008 and ops sections, but implementation status and v0.3 target unconfirmed. (src: 02-architecture.md, 06-decisions.md, 07-operational.md)

- **Defragmentation policy for soft-deleted vectors**: Marked "lazy, not automatic in 0.2.x" but no public API or roadmap for manual trigger beyond `clear()`. When should users defrag? What triggers compaction in practice? (src: 04-data-and-storage.md, 06-decisions.md)

- **Multi-GPU context management**: Documented as "manual in 0.2.x" but no examples or helper traits for distributing vectors across GPUs. API shape for multi-GPU unknown. (src: 08-cortex-relevance.md)

- **Async/tokio integration**: Decision D011 forbids async in core; examples use Tokio. Unclear how applications should integrate HiveGPU with async runtimes (pinning, execution model). (src: 06-decisions.md)

## API Stability & Versioning

- **HNSW API surface (v0.4)**: Mentioned as "subject to HNSW/quantization additions" but no concrete trait design or breaking-change plan. Will `GpuContext` and `GpuVectorStorage` be extended or new traits added? (src: 01-overview.md, 03-public-surface.md)

- **Quantization format (v0.4)**: Planned as "PQ/SQ" but no spec for storage layout, recovery, or compatibility with Expert's learned codebooks. Will it be opt-in or required? (src: 01-overview.md, 05-integrations.md)

- **Serialization roadmap**: D010 defers checkpoint/restore for IvfIndex to "usage patterns clarify". No acceptance criteria for when snapshots are needed. (src: 06-decisions.md)

## Hardware & Testing

- **ROCm backend validation timeline**: Documented as "code-complete, pending hardware validation (phase4e)" but no blockers, dependencies, or expected completion date. What hardware? What tests? (src: 01-overview.md)

- **Intel/Vulkan backend validation**: Same status as ROCm; no timeline. Can CI run on non-Intel GPUs with `HIVE_GPU_VULKAN_UNIVERSAL=1`? Untested. (src: 01-overview.md, 07-operational.md)

- **CPU fallback performance baseline**: Documented as "expected 1–10× slower" but no measured data on specific hardware. How is fallback tested in CI without GPUs? (src: 07-operational.md)

## Integration & Consumption

- **Cortex integration effort / priority**: Estimated as "~2–3 days" for Phase 1 and "~1 week" for Phase 3 (IVF), but no dependencies, blockers, or task breakdown. Is this relative to Cortex roadmap? (src: 08-cortex-relevance.md)

- **Expert GPU sharing**: Documented as "future relevance" for v0.4; unclear if Expert and HiveGPU can coexist on same GPU or if orchestration is required. (src: 05-integrations.md)

- **Nexus node embedding loop closure**: Noted as "potential confluence (future)" but no concrete proposal. How would Cortex link embeddings back to Nexus graph? Cortex responsibility or HiveGPU? (src: 05-integrations.md)

## Operations & Reliability

- **VRAM OOM handling for Cortex**: Success criteria includes "GPU OOM gracefully degrades to CPU" but no mechanism in HiveGPU or Cortex layer. Who catches the error? How is fallback triggered? (src: 08-cortex-relevance.md)

- **Multi-tenant isolation**: No discussion of GPU memory quotas, process isolation, or fairness if multiple services share one GPU. Is this out-of-scope? (src: 04-data-and-storage.md, 07-operational.md)

- **VRAM monitoring integration with Synap**: Listed as "operational value (future)" but no specific metrics, schemas, or SLA. What should Cortex observe? (src: 05-integrations.md)

## Performance & Tuning

- **Search latency profile at scale**: Benchmarks provided for RTX 4090 and M3 Pro, but no data for ROCm or Intel. Are performance characteristics portable across vendors? (src: 04-data-and-storage.md)

- **IVF parameter sensitivity**: Tuning table recommends ranges (n_list 50–500, nprobe 5–50) but no guidance on choosing for Cortex's specific workload (doc length, embedding dim, query volume). (src: 07-operational.md, 08-cortex-relevance.md)

- **Batch size recommendation for GPU win**: Doc says "use ≥10K vectors" but assumes SGEMV launch overhead. Varies by GPU backend? Measured on which hardware? (src: 07-operational.md)

## Documentation & Examples

- **No Cortex-specific integration example**: 03-public-surface.md shows Metal/CUDA quick-starts, but no pattern for embedding-store wrapper or storage trait impl. Where should Cortex begin? (src: 03-public-surface.md)

- **HNSW roadmap details**: Planning doc mentions v0.4 but no PR, task, or issue link. How can users follow progress or contribute? (src: 01-overview.md)

- **Crate feature matrix**: No table showing which features compile on which platforms (e.g., metal-native only on macOS) or which CI workflows test combinations. (src: 02-architecture.md, 07-operational.md)
