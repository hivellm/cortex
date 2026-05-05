# HiveGPU — Operations, Deployment & Runtime

## Build & Development

### Prerequisites

- **Rust 1.70+** (MSRV)
- **Platform-specific drivers** (runtime only; build-time optional):
  - **macOS**: Xcode command-line tools (Metal is built-in)
  - **Linux/Windows + NVIDIA**: NVIDIA driver (no CUDA Toolkit required; cudarc uses dynamic linking)
  - **Linux + AMD**: ROCm 6.x runtime (libamdhip64.so + librocblas.so on linker path)
  - **Linux/Windows + Intel/Vulkan**: Vulkan 1.2 loader (libvulkan.so.1 or vulkan-1.dll)

### Build Commands

```bash
# Metal (macOS) — default feature
cargo build --features metal-native

# CUDA (Linux/Windows with NVIDIA driver)
cargo build --features cuda

# ROCm (Linux with ROCm runtime)
cargo build --features rocm

# Intel/Vulkan (Linux/Windows)
cargo build --features intel --build-override=naga # forces WGSL → SPIR-V at build

# All backends
cargo build --features metal-native,cuda,rocm,intel

# Check all feature combinations
cargo check --all-targets
```

### CI/CD

- **GitHub Actions**: `.github/workflows/cuda-build.yml`
  - Runs against `nvidia/cuda:12.4.1-devel-ubuntu22.04` container
  - Tests: `cargo test --features cuda`
  - Checks: clippy with `-D warnings`, fmt

## Runtime Configuration

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `HIVE_GPU_BACKEND` | (auto-detect) | Force backend: `metal`, `cuda`, `rocm`, `intel`, `cpu` (planned) |
| `HIVE_GPU_VULKAN_UNIVERSAL` | (unset) | Set to `1` to fallback Intel Vulkan on non-Intel GPUs |
| `RUST_LOG` | (unset) | Set to `debug` / `trace` for verbose GPU operation logging |

### GPU Runtime Requirements

**CUDA (NVIDIA)**:
- Driver version: 450+ (compatible with CUDA 12.x)
- Compute capability: sm_70+ (Volta or newer)
- VRAM: Depends on vector dimension; recommend 8 GB+ for production workloads

**ROCm (AMD)**:
- ROCm version: 6.0+
- Architecture: gfx900 (Polaris XT), gfx1030 (RDNA 2), gfx1100 (RDNA 3+)
- VRAM: 8 GB+ for 1M+ vectors

**Metal (Apple Silicon)**:
- macOS: 12.0+ (13.0+ recommended)
- Chip: M1 or later (Pro/Max/Ultra)
- Unified memory: No explicit allocation (hardware-managed)

**Intel/Vulkan**:
- Vulkan 1.2 driver
- Intel Arc A380+, or any Vulkan 1.2 GPU
- VRAM: Device-dependent

### CPU Fallback

When no GPU available, all backends fall back to scalar CPU loops:
- No performance guarantee (expected 1–10× slower than GPU)
- Identical API surface; transparent to application
- Useful for development/testing on CPU-only hosts

## Monitoring & Diagnostics

### Device Info API

Query GPU info at runtime:

```rust
let context = CudaContext::new()?;
let device_info = context.device_info()?;

println!("Device: {}", device_info.name);              // "NVIDIA RTX 4090"
println!("Backend: {}", device_info.backend);          // "cuda"
println!("Compute: {}", device_info.compute_capability); // "8.9"
println!("Total VRAM: {} MB", device_info.total_vram_mb()); 
println!("Used VRAM: {} MB", device_info.used_vram_mb());
println!("Usage: {:.1}%", device_info.vram_usage_percent());
```

### VRAM Monitoring

Per-backend VRAM tracker:
- `GpuMemoryStats`: total, used, available
- `GpuMonitor::check_vram()`: validate before large allocations
- Soft-delete tracking: `deleted_indices` set prevents reallocation on deletes

### Performance Profiling

Criterion benchmarks in `benches/`:

```bash
# CUDA benchmarks
cargo bench --features cuda --bench cuda_ops

# Metal benchmarks
cargo bench --features metal-native --bench gpu_operations

# IVF-specific (CUDA)
cargo bench --features cuda --bench cuda_ivf
```

Outputs HTML reports in `target/criterion/`.

## Containerization

No official Docker image. Consumers embed HiveGPU in their own image:

```dockerfile
FROM nvidia/cuda:12.4.1-devel-ubuntu22.04

WORKDIR /app

# Copy app using HiveGPU
COPY . .

# Build (CUDA feature)
RUN cargo build --release --features cuda

# Runtime: ensure NVIDIA driver is available at runtime
RUN nvidia-smi  # validate GPU access
```

## Testing Strategy

All test suites are no-ops on hosts without the target GPU (green CI on GPU-less runners):

```bash
# Metal (macOS only)
cargo test --features metal-native --lib --tests

# CUDA (skipped if no NVIDIA driver)
cargo test --features cuda

# ROCm (skipped if no ROCm runtime)
cargo test --features rocm

# Intel (skipped if no Vulkan GPU)
cargo test --features intel --test intel_smoke
```

Each backend's tests:
- Device discovery (no-op if device unavailable)
- Kernel correctness (tolerance 1e-3 to 1e-5 depending on metric)
- IVF recall (≥0.95 on clustered data)
- Memory management (adaptive growth, soft-delete)

## Performance Tuning

| Parameter | Knob | Recommendation |
|-----------|------|-----------------|
| **SGEMV launch overhead** | Batch size | Use ≥10K vectors for GPU advantage |
| **IVF n_list** | Cluster count | 50–500; higher → more clusters, smaller search |
| **IVF nprobe** | Top clusters | 5–50; higher → recall 0.95+, but slower search |
| **Buffer growth factor** | 2× / 1.5× / 1.2× | Fixed; no tuning required |
| **Soft-delete threshold** | Compaction trigger | Manual in 0.2.x (run `clear()` to force) |

## Version / Release Cycle

- **Current**: 0.2.0 (2026-04-19)
- **Patch (0.2.x)**: Hardware validation of blind backends (Metal, ROCm, Intel)
- **Minor (0.3)**: HNSW + quantization
- **Breaking**: None planned in 0.x series
