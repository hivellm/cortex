# HiveGPU — Public APIs, SDKs, CLIs

## Rust SDK (Primary)

Published as [hive-gpu](https://crates.io/crates/hive-gpu) on crates.io.

### Core Traits (Public API)

```rust
pub trait GpuContext {
    fn create_storage(dimension: usize, metric: GpuDistanceMetric) 
        -> Result<Box<dyn GpuVectorStorage>>;
    fn create_storage_with_config(dimension, metric, config: HnswConfig) 
        -> Result<...>;
    fn device_info() -> Result<GpuDeviceInfo>;
    fn memory_stats() -> GpuMemoryStats;
}

pub trait GpuVectorStorage {
    fn add_vectors(&mut self, vectors: &[GpuVector]) -> Result<Vec<usize>>;
    fn search(&self, query: &[f32], limit: usize) -> Result<Vec<GpuSearchResult>>;
    fn remove_vectors(&mut self, ids: &[String]) -> Result<()>;
    fn vector_count(&self) -> usize;
    fn dimension(&self) -> usize;
    fn get_vector(&self, id: &str) -> Result<Option<GpuVector>>;
    fn clear(&mut self) -> Result<()>;
}

pub trait GpuBackend {
    fn device_info(&self) -> GpuDeviceInfo;
    fn supports_operations(&self) -> GpuCapabilities;
    fn memory_stats(&self) -> GpuMemoryStats;
}
```

### Core Types

```rust
pub struct GpuVector {
    pub id: String,
    pub data: Vec<f32>,           // always f32
    pub metadata: HashMap<String, String>,
}

pub enum GpuDistanceMetric {
    Cosine,
    Euclidean,
    DotProduct,
}

pub struct GpuSearchResult {
    pub id: String,
    pub score: f32,
    pub index: usize,
}

pub struct GpuDeviceInfo {
    pub name: String,                     // e.g. "NVIDIA RTX 4090"
    pub backend: String,                  // e.g. "cuda"
    pub total_vram_bytes: u64,
    pub available_vram_bytes: u64,
    pub used_vram_bytes: u64,
    pub compute_capability: Option<String>, // CUDA: "8.9", Metal: "Apple Silicon", etc.
    pub driver_version: Option<String>,
}

pub struct HnswConfig {
    pub max_connections: u32,
    pub ef_construction: usize,
    pub ef_search: usize,
}

pub struct IvfConfig {
    pub n_list: usize,
    pub nprobe: usize,
    pub kmeans_iterations: usize,
}
```

### Backend Contexts (Platform-Specific)

- `hive_gpu::metal::context::MetalNativeContext` → `GpuContext` (macOS)
- `hive_gpu::cuda::CudaContext` → `GpuContext` (Linux/Windows, NVIDIA)
- `hive_gpu::rocm::context::RocmContext` → `GpuContext` (Linux, AMD)
- `hive_gpu::intel::context::IntelContext` → `GpuContext` (Linux/Windows, Intel Arc/Vulkan)

### Examples

Runnable examples in `examples/`:
- `metal_basic.rs` — Metal backend quick start
- `cuda_basic.rs` — CUDA backend quick start

### Benchmarks

- `benches/cuda_ops.rs` — CUDA search latency vs CPU (Criterion)
- `benches/cuda_ivf.rs` — CUDA IVF build time + search recall
- `benches/gpu_operations.rs` — Metal operations benchmark

## Documentation

- **[Main README](../../README.md)**: Quick start, installation, feature flags
- **[API Reference](docs/reference/API_REFERENCE.md)**: Full method signatures
- **[Integration Guide](docs/guides/INTEGRATION_GUIDE.md)**: Vectorizer integration, custom databases
- **[Performance Guide](docs/benchmarks/PERFORMANCE.md)**: Tuning, hardware matrix

## CLI / Tools

No standalone CLI. HiveGPU is consumed as a library only.

## Deployment / Container

No official Docker image. Consumers embed HiveGPU in their own containers.

## Version / Release

- **Current**: 0.2.0 (2026-04-19)
- **Crate metadata**: docs.rs, GitHub Actions CI, CHANGELOG.md
- **Stability**: Beta; API surface stable; quantization/HNSW to follow in 0.4
