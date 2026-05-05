# Expert System Design Decisions & Rationale

## D1: Not Mixture-of-Experts (MoE)

**Decision:** Build expert composition on external adapters (LoRA/DoRA/IA³), not internal MLP routing.

**Rationale:**
- **Extensibility:** Can add experts after training without retraining base model
- **Distribution:** Each expert is 5-80MB, distributable via Git (vs. monolithic 100GB+ model)
- **Specialization cost:** $15-50 per expert with synthetic data (vs. $1M+ for MoE retraining)
- **Control:** Explicit heuristic/embedding-based selection (vs. learned gating per-token)
- **VRAM efficiency:** Only load experts you need (vs. all experts in model)

**Tradeoff:** Per-query selection overhead (~15-50ms router latency) vs. per-token automatic selection

## D2: Qwen3-0.6B as Base Model

**Decision:** Use compact 0.6B model rather than larger foundation.

**Rationale:**
- **Fits 8GB VRAM:** INT4 quantized = 0.3-0.6GB, leaves 7+ GB for 10 experts + KV cache
- **Good quality-to-size ratio:** Outperforms older 3.5B+ models on domain tasks when specialized
- **Training time:** Experts train in hours, not weeks
- **Inference speed:** ~100-150 tok/s on RTX 4090 (acceptable for batch processing)

**Tradeoff:** Less general knowledge vs. larger base models; mitigated by expert composition

## D3: Git-Based Marketplace (No NPM/PyPI)

**Decision:** Experts distributed via Git repositories, not centralized package registry.

**Rationale:**
- **No gatekeeping:** Fork, modify, share instantly; no approval process
- **Version control:** Git tags = semantic versioning; natural branching for variants
- **Discovery:** Community can search, star, fork on GitHub (vs. custom registry UI)
- **Signing:** Ed25519 signatures per package maintain trust without central authority
- **No lock-in:** Can mirror, fork, host anywhere

**Tradeoff:** Must implement Git cloning in Rust; no centralized discoverability (mitigated by search indexing in Vectorizer)

## D4: Python for Training, Rust for Runtime

**Decision:** Hybrid architecture with clear separation.

**Rationale:**
- **Training (Python):** ML ecosystem unbeatable (PyTorch, PEFT, TRL, datasets, safetensors)
- **Runtime (Rust):** Low latency, single binary, memory safety, no GC pauses
- **Production safety:** Inference binary is pure Rust (no Python GIL contention)
- **Deployment:** Single compiled binary, deployable to any system

**Tradeoff:** Two languages; mitigated by clear interface (manifest.json + .expert packages)

## D5: Paged KV Cache (Not Pooled)

**Decision:** Per-job isolated KV cache with paging strategy.

**Rationale:**
- **Isolation:** Different expert sets can't share cache (incompatible compositions)
- **Efficiency:** Paging prevents fragmentation for long contexts (up to 200k tokens)
- **Multi-job:** Multiple concurrent jobs get independent KV budgets
- **vLLM-proven:** Works well in production (vLLM paper validates approach)

**Tradeoff:** More complex implementation vs. shared KV (which isn't viable with hot expert swapping)

## D6: LoRA-First Adapter Strategy

**Decision:** Prefer LoRA over IA³ despite size, offer DoRA as premium option.

**Rationale:**
- **Maturity:** LoRA is proven, stable, widely supported (papers, tutorials, tools)
- **Quality:** Consistent results across diverse domains (SQL, Cypher, JSON, etc.)
- **Expressiveness:** Rank 16-32 captures domain-specific patterns effectively
- **Future:** Can auto-merge heavy-use experts for further optimization

**Support:**
- LoRA: Primary (10-80MB at rank 16)
- LoRA-FA: Lightweight variant (half params)
- DoRA: Premium option for quality-critical experts (slight size/compute increase)
- IA³: Ultra-lightweight for resource-constrained scenarios (<1% use case)

## D7: Max 10 Experts Per Inference

**Decision:** Hard limit on concurrent expert loading.

**Rationale:**
- **VRAM budget:** 10 × 25MB avg = 250MB; leaves headroom for KV cache + system
- **Router complexity:** Beyond 10, selection accuracy drops (token budget, inference time)
- **Composition semantics:** Beyond 5-6, interaction effects unpredictable
- **Practical:** Most tasks use 2-4 experts; 10 covers 99.9% of use cases

## D8: RoPE Scaling for 200k Context

**Decision:** Use YaRN (Yet another RoPE extensioN) over simpler NTK.

**Rationale:**
- **Better interpolation:** YaRN handles attention scaling better than raw frequency interpolation
- **Empirical:** Better eval scores on long-context benchmarks
- **Future-proof:** Can stretch to 256k if needed

**Baseline:** Qwen3-0.6B trained on 128k; YaRN stretch to 200k with minimal quality loss

## D9: Synchronous Installation (No Async Package Pulls)

**Decision:** Expert installation blocks until download + signature verification complete.

**Rationale:**
- **Safety:** Prevents partial installation state
- **Atomicity:** Registry updated only after full validation
- **Simplicity:** No background job tracking needed (user expects install → ready)

**Mitigated:** Most installs are <1 min for 25-30MB packages on decent connectivity
