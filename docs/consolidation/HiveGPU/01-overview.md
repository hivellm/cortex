# HiveGPU — Overview

**HiveGPU** is a high-performance GPU acceleration library for vector operations, specifically vector similarity search. Written in Rust, it provides a unified trait-based API across four native GPU backends with optimized algorithms.

## Purpose

Enable GPU-accelerated vector search on diverse hardware platforms:
- **Vector similarity indexing**: brute-force and IVF (Inverted File) search
- **Distance metrics**: Cosine, Euclidean, DotProduct
- **HNSW (Hierarchical Navigable Small World)**: graph-based approximate nearest neighbor search (planned v0.4)

## Role in HiveLLM Ecosystem

HiveGPU accelerates **embedding search** workloads in Cortex and other downstream services. Designed to integrate with Hive-Vectorizer for GPU-backed vector operations without reimplementing embedding services.

## Stack

- **Language**: Rust (edition 2021)
- **Runtime backends**:
  - **Metal (native objc2)**: Apple Silicon / macOS (stable, validated 0.1.x)
  - **CUDA (cudarc driver API + cuBLAS)**: NVIDIA Volta/sm_70+ on Linux/Windows (shipped 0.2.0, validated RTX 4090)
  - **ROCm/HIP (hand-rolled FFI + rocBLAS)**: AMD gfx900–gfx1100 on Linux (code-complete, pending hardware validation)
  - **Intel/Vulkan Compute (ash + WGSL→SPIR-V)**: Intel Arc / Battlemage on Linux/Windows (code-complete, pending validation)
- **CPU fallback**: Scalar reference implementations in all backends

## Maturity

- **Version**: 0.2.0 (released 2026-04-19)
- **Production status**: Beta
  - Metal: shipped (0.1.x)
  - CUDA: shipped (0.2.0), validated end-to-end on RTX 4090
  - ROCm, Intel: code-complete, authored blind, pending hardware validation (phase4e, phase4f)
- **API stability**: Core traits (`GpuContext`, `GpuVectorStorage`, `GpuBackend`) stable; subject to HNSW/quantization additions in v0.4

## Roadmap Milestones

- **v0.2.x**: Hardware validation of Metal, ROCm, Intel backends (phase4d, phase4e, phase4f)
- **v0.4**: GPU HNSW construction + search, quantization (PQ/SQ), GPU top-K (radix select)
