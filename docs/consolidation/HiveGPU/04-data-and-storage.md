# HiveGPU — Data Model & Storage

## Vector Format

**Immutable, single-type representation**:

```rust
pub struct GpuVector {
    pub id: String,                            // unique vector identifier
    pub data: Vec<f32>,                        // always f32, CPU-side slice
    pub metadata: HashMap<String, String>,     // application-specific tags
}
```

- **Dimensionality**: Fixed at storage creation; all vectors must match
- **Element type**: f32 only (no f16 quantization in 0.2.x; SQ/PQ planned v0.4)
- **Memory on CPU**: `id` (String) + vector slice + metadata map
- **Memory on GPU**: Contiguous f32 buffer allocated and managed by backend

## GPU Storage Models

### Brute-Force Storage

Each backend maintains a single contiguous GPU buffer:

- **CUDA** (`CudaVectorStorage`): Single `CudaSlice<f32>` on device
  - Adaptive capacity growth: 2× → 1.5× → 1.2×
  - Batched htod_copy + dtod_sync memcpy for updates
  - Soft-delete via `HashSet<usize>` of deleted indices
  - Host-cached squared norms for distance computation

- **Metal** (`MetalVectorStorage`): MTLBuffer pool
  - Dynamic growth mirrors CUDA shape
  - Compute shader dispatch for SGEMV (search)

- **ROCm** (`RocmVectorStorage`): HIP device buffer
  - Architecture mirrors CUDA exactly
  - rocBLAS SGEMV + SGEMM kernels

- **Intel** (`IntelVectorStorage`): Vulkan buffer
  - WGSL compute shaders compiled to SPIR-V at build
  - Vulkan 1.2 device memory management

### IVF Index Storage

All backends implement `IvfIndex` with shared data structures:

- **Cluster centroids**: `n_list` × `dimension` f32 matrix (GPU-side)
- **Cluster assignments**: Vector → cluster index mapping
- **Inverted lists**: Per-cluster residual vectors + IDs
- **Configuration**: `n_list`, `nprobe`, `kmeans_iterations`

**Construction**:
1. k-means++ initialization: select initial centroids from data
2. Lloyd iterations: assign → recompute centroids
3. Residual refinement: store `vector - centroid[assigned_cluster]` per cluster

**Search**:
1. Compute distances query → all centroids (SGEMV)
2. Select top `nprobe` nearest clusters
3. Linear search within selected clusters
4. Merge results, return top-k

## Memory Management

### Allocation

- **GPU buffer pool**: Pre-allocate and reuse across add/remove operations
- **Staged buffers**: Host-to-device copy via staging when needed
- **Device-to-device copy**: For in-place reallocation (growing vectors)

### Adaptive Growth

When capacity exceeded:
1. Try 2× growth
2. If fails (VRAM exhausted), try 1.5×
3. If fails, try 1.2×
4. If all fail, return out-of-memory error

### Soft Delete

Deleted vectors are not immediately removed from GPU buffer; instead:
- Mark index in `deleted_indices` set
- Skip marked indices during search
- Periodically defragment (lazy, not automatic in 0.2.x)

## Distance Metrics

Supported on all backends:

1. **Cosine**: `cos(u, v) = (u · v) / (|u| · |v|)` — normalized dot product
2. **Euclidean**: `L2(u, v) = sqrt(sum((u[i] - v[i])^2))` — derived from dot products + norms
3. **DotProduct**: `u · v` — raw inner product

All computed via cuBLAS SGEMV (CUDA), Metal compute kernels, rocBLAS (ROCm), or WGSL (Intel).

## Serialization

- **Vector types**: `serde` support (Serialize/Deserialize) for `GpuVector`, `GpuDistanceMetric`, `GpuSearchResult`
- **Index state**: No built-in checkpoint; applications must implement persistence
- **IvfIndex**: In-memory only (no snapshot/load in 0.2.x)

## Performance Characteristics

| Operation | CUDA RTX 4090 (128-dim) | Notes |
|-----------|------------------------|-------|
| Add 10K vectors | 7.1 ms | 1.41M elem/s throughput |
| Search 1K vectors, top-10 | 124 µs | Brute-force; host copy dominates |
| Search 100K vectors, top-10 | 4.01 ms | GPU 3.25× over CPU |
| IVF build (1M vectors) | ~seconds | k-means || iterations + cluster assignment |
| IVF search at nprobe=10 | ~100 µs | Cached centroid query + per-cluster scan |

Metal performance (Apple M3 Pro):
- Search throughput: 1.08M qps (massive speedup due to low latency, cache effects)
- Add vectors: 3.7–4.25× over CPU
