# HiveGPU — Architecture

## Layered Design

```
Application (Cortex, Vectorizer, custom code)
    ↓
Core API (traits + types)
    ↓
Backend Abstraction (device detection, buffer pool)
    ↓
GPU Implementations (Metal / CUDA / ROCm / Intel)
    ↓
Hardware (GPU device + driver)
```

## Core Modules

### Foundation Layer

**`src/types.rs`** — immutable data model:
- `GpuVector`: ID, f32 slice, metadata
- `GpuDistanceMetric`: Cosine, Euclidean, DotProduct
- `GpuSearchResult`: ID, score, index
- `GpuDeviceInfo`: name, VRAM (total/available/used), backend, compute capability
- `HnswConfig`, `IvfConfig`: algorithm parameters

**`src/error.rs`** — error types:
- `HiveGpuError` enum with variants per backend: `CudaError`, `HipError`, `VulkanError`, etc.
- Result<T> wrapper

### Abstraction Layer

**`src/traits.rs`** — backend-agnostic contracts:
- `GpuBackend`: device info, capabilities, memory stats
- `GpuContext`: factory for creating storage instances with/without HNSW config
- `GpuVectorStorage`: add, search, remove, clear vectors; vector count/dimension; get by ID
- `GpuBufferManager`: allocate, deallocate, resize buffers
- `GpuMonitor`: VRAM validation and monitoring

### Backend Detection

**`src/backends/detector.rs`**:
- Runtime backend probing: Metal > CUDA > ROCm > Intel > CPU
- `is_metal_available()`, `is_cuda_available()`, `is_rocm_available()`, `is_intel_available()`
- Override via `HIVE_GPU_BACKEND` env var (planned)

### GPU Implementation Modules

Each backend (`metal/`, `cuda/`, `rocm/`, `intel/`) mirrors the same structure:

- **`context.rs`**: Device initialization, device info querying, context state
- **`vector_storage.rs`**: Storage implementation — buffer allocation, add/search/remove ops
- **`ivf.rs`**: Inverted-File index — k-means++ clustering, Lloyd iterations, per-list search
- **`buffer_pool.rs`**: Buffer lifecycle — allocation, reuse, adaptive growth (2× / 1.5× / 1.2×)
- **`vram_monitor.rs`**: VRAM tracking, validation, soft-delete tracking
- **`helpers.rs`**: GPU-specific utilities (compute kernel dispatch, shader compilation, etc.)
- **`ffi.rs`** (ROCm only): Hand-rolled HIP FFI via libloading

### Compute Kernels

**`src/shaders/`**:
- **Metal**: `sgemv_dot.metal`, `sgemm_dot.metal` — compute shaders for SGEMV (search) + SGEMM (IVF assignment)
- **Intel/WGSL**: `sgemv_dot.wgsl`, `sgemm_dot.wgsl` — compiled to SPIR-V at build time via `naga`
- **CUDA**: Embedded PTX kernels (offline-compiled); uses cuBLAS SGEMV + SGEMM for search
- **ROCm**: rocBLAS SGEMV + SGEMM via hand-rolled HIP FFI

### Monitoring & Utils

**`src/monitoring/`**: Performance monitoring, timing, throughput tracking
**`src/utils/`**: Math utilities, memory helpers, timing

## Data Flow

1. **Initialization**: Application calls `context.create_storage(dim, metric)`
2. **Add vectors**: `storage.add_vectors(&[GpuVector, ...])` → allocate/reuse GPU buffer → htod_copy
3. **Search**: `storage.search(&query, limit)` → dispatch GPU kernel → read results back to CPU
4. **Index (IVF optional)**: `storage.create_ivf_index()` → k-means clustering → residual refinement per cluster

## Feature Flags

- `metal-native`: macOS Metal (default)
- `cuda`: NVIDIA CUDA
- `rocm`: AMD ROCm/HIP
- `intel`: Intel Arc / Vulkan Compute

All gated to platform; crate builds cleanly on any host.
