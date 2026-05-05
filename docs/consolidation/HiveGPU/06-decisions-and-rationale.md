# HiveGPU — Design Decisions & Rationale

## D001: Trait-Based Abstraction over Backends

**Decision**: Implement `GpuContext`, `GpuVectorStorage`, `GpuBackend` traits; each platform provides a concrete type.

**Rationale**:
- Enables runtime backend selection without recompilation
- Unified API across Metal/CUDA/ROCm/Intel
- Allows CPU fallback without code duplication
- Simplifies testing (trait mocks possible)

**Trade-off**: Slight dyn dispatch overhead (mitigated by batch operations).

## D002: Feature-Gated Backends

**Decision**: Use Cargo features `metal-native`, `cuda`, `rocm`, `intel`; platform-gated with `cfg()`.

**Rationale**:
- Crate builds cleanly on any host (e.g., Linux without NVIDIA driver)
- Dependencies are optional; smaller binaries when unused
- Each feature pulls only necessary deps (cudarc, ash, naga, objc2-metal)

**Implication**: CI must test all feature combinations; separate workflows for each platform.

## D003: Single f32 Element Type (No Heterogeneous Dtypes)

**Decision**: All vectors are `Vec<f32>` on CPU and GPU. No f16, f64, int8 in 0.2.x.

**Rationale**:
- Simplifies kernel code (no template dispatch)
- Matches cuBLAS/rocBLAS native f32 speed (highest performance tier)
- Quantization (f16/int8) deferred to v0.4 as PQ/SQ

**Trade-off**: Higher memory usage; mitigated by quantization roadmap.

## D004: Adaptive Buffer Growth (2× → 1.5× → 1.2×)

**Decision**: Grow GPU buffer with exponential backoff when capacity exceeded.

**Rationale**:
- Avoids allocation failure on first VRAM exhaustion
- Reduces fragmentation (fewer large allocs than realloc-every-vector)
- Matches Metal backend empirical tuning

**Details**:
1. If 2× reallocation fails, try 1.5× → 1.2× → fail
2. Soft-delete indices remain valid (no defragmentation on grow)

## D005: Soft-Delete with Deferred Compaction

**Decision**: Deleted vectors marked in a HashSet; not immediately removed from GPU buffer.

**Rationale**:
- O(1) delete latency (no GPU memcpy)
- Skip marked indices during search (O(n) scan cost minimal if deletions sparse)
- Deferred compaction avoids thrashing GPU memory

**Trade-off**: Memory waste if deletions are heavy; no automatic defrag in 0.2.x.

## D006: Host-Cached Norms for Distance Computation

**Decision**: Pre-compute and cache L2 norms on CPU; use for Cosine/Euclidean derivation.

**Rationale**:
- Avoids redundant norm computation during search
- Cosine: `cos(u,v) = (u·v) / (||u|| · ||v||)` can reuse cached norms
- Euclidean: `L2(u,v) = sqrt(u·u + v·v - 2*u·v)` uses cached terms

**Implementation**: Stored in `CudaVectorStorage::squared_norms: Vec<f32>`.

## D007: Kernel Strategy: cuBLAS + Custom Shaders

**Decision**: Use optimized BLAS (cuBLAS, rocBLAS) for SGEMV/SGEMM; write custom shaders for compute kernels (Metal, Intel).

**Rationale**:
- BLAS libraries highly optimized; no need to reimplement
- Custom shaders for operations BLAS doesn't cover (e.g., Metal compute, Vulkan dispatch)
- Intel: WGSL compiled to SPIR-V at build time via naga (pure-Rust, no C++ toolchain)

**Trade-off**: Maintenance burden for custom kernels; mitigated by reference against CUDA kernels.

## D008: Runtime Backend Detection Order: Metal > CUDA > ROCm > Intel > CPU

**Decision**: Probe in priority order; select first available.

**Rationale**:
- Metal: fastest on Apple Silicon (lowest latency, shared memory)
- CUDA: highest market share (~70% ML/AI GPUs)
- ROCm: covers AMD (~15% market)
- Intel: fallback for Arc / Vulkan-capable devices
- CPU: universal fallback

**Flexibility**: Override via `HIVE_GPU_BACKEND` env var (planned v0.3).

## D009: IVF Index as Separate Optional Type

**Decision**: `IvfIndex` trait implemented per backend; created via `create_ivf_index()`.

**Rationale**:
- Brute-force is the simple case; IVF adds complexity
- Optional: applications can choose speed/memory trade-off
- Enables future HNSW without breaking brute-force API

**API**: Separate `.create_ivf_index()` method; not automatic.

## D010: Serialization Deferred (0.2.x)

**Decision**: Core types implement `serde` (Serialize/Deserialize); index state does not persist in 0.2.x.

**Rationale**:
- Simple types serde-enabled; vectors can be JSON-serialized
- IvfIndex checkpoint deferred until usage patterns clarify
- Applications can implement custom persistence if needed

**Road-out**: v0.4 may add snapshot/restore for IvfIndex.

## D011: No Async Runtime Dependency

**Decision**: Core library is `tokio`-free. Examples use Tokio; tests do not require it.

**Rationale**:
- GPU operations are already blocking/synchronous
- Async would complicate integration (pinned buffers, cancellation)
- Consumers can spawn tokio tasks around sync HiveGPU calls

**Exception**: `examples/` use Tokio for convenience; not required.
