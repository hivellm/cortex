# Expert System Overview

## Purpose & Role in HiveLLM

Expert is a dynamic expert composition system that enables specialized AI inference on consumer hardware (8-16GB VRAM). It runs lightweight, task-specific adapter modules (LoRA/DoRA/IA³) on top of a compact base model (Qwen3-0.6B) without ever merging adapters into the base model weights.

**Role in HiveLLM ecosystem:**
- Provides specialized inference capabilities for downstream systems (Cortex, Synap, etc.)
- Solves the GPU-cost problem: replaces need for massive models (70B+) with dynamic composition
- Enables marketplace distribution of experts via Git repositories (no NPM/PyPI centralization)

## Project Maturity

**Status**: CLI implementation phase (15% overall, design 100% complete)

**Completed:**
- Full architecture documentation suite
- Expert training pipelines (Python + PyTorch/PEFT/TRL)
- Expert packaging system (.expert tar.gz format)
- Rust inference runtime skeleton with Candle + CUDA support
- LoRA/DoRA adapter merging (168-weight merging)
- First wave of domain experts (relational queries, graph analytics, JSON)
- Comprehensive test coverage (5 automated test scripts)

**In Progress:**
- Multi-expert routing and composition algorithms
- Router intelligent selection logic (heuristics + embeddings + mini-policy)
- Marketplace CLI (`install`, `verify`, `list`, signature verification)

## Core Tech Stack

**Python (Training & Tooling)**
- PyTorch 2.5.1+cu121, PEFT 0.17+ (LoRA/IA³/DoRA), TRL 0.7+ (SFTTrainer)
- Transformers 4.57+, BitsAndBytes 0.48+ (INT4/INT8 quantization)
- LLaMA-Factory/Unsloth-inspired training optimizations

**Rust (Runtime & Production)**
- Candle (HuggingFace) for GPU tensor ops, SafeTensors for weight loading
- Edition 2024, CUDA 12.1+ support, Tokio async runtime
- Single-binary deployment with PyO3 bindings (Python) and future NAPI (Node)

**Base Model**
- Qwen3-0.6B quantized (INT4/INT8), ~0.3-0.6GB VRAM
- Context: 120k-200k tokens via RoPE scaling (YaRN/NTK)
- Supports paged KV cache for efficient long-context inference

## Hardware Requirements

- **GPU**: NVIDIA RTX 3060+ (8GB minimum), RTX 4090 recommended
- **CUDA**: 12.1+ (tested on Windows + Linux)
- **RAM**: 16GB+ system memory
- **Storage**: 50GB+ SSD for base model + experts
- **OS**: Windows 10/11, Linux (Ubuntu 22.04+)
