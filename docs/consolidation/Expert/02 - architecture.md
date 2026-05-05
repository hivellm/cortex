# Expert System Architecture

## Six Core Components

### 1. Base Model (MB)
- **Model**: Qwen3-0.6B quantized (INT4 preferred, INT8 for complex reasoning)
- **VRAM**: 0.3-0.6GB (quantized), 1.2GB (FP16)
- **Context**: 128k-256k tokens with RoPE scaling (YaRN/NTK)
- **Attention**: Paged (vLLM-style for long context)
- **Permanence**: Fixed in GPU VRAM for entire session

### 2. Experts (EXPs)
Lightweight adapters never merged into base model. By preference order:

1. **LoRA** (10-80 MB): Low-rank adaptation, rank 8-32 (sweet spot: 16)
2. **LoRA-FA**: Frozen-A variant, half trainable params
3. **DoRA** (Weight-Decomposed LoRA): Better quality than LoRA, slightly higher compute
4. **IA³** (1-5 MB): Element-wise scaling vectors, extremely lightweight
5. **Soft Prompts** (<1 MB): Learned embeddings prepended to input
6. **Custom Vocab Heads** (rare): Domain-specific tokens

**Composition**: Additive (multiple experts apply per layer via summation or element-wise operations)

### 3. Router/Reasoning (RG)
CPU-based decision engine running in parallel with previous inference.

**Process:**
1. Feature extraction (heuristics ~1ms, embeddings ~10ms, mini-policy ~50ms)
2. Expert index query (ANN search via Vectorizer/FAISS)
3. Scoring and filtering (semantic relevance, VRAM cost, success rate)
4. Top-K selection (K ≤ 10)
5. Parameter tuning (temperature, max_tokens, top-p, repetition penalty)

**Output**: ExpertPlan {experts, order, params}

### 4. Inference Runtime (RI)
GPU execution engine with hot-swap adapter support.

**Responsibilities:**
- Load/unload experts in <10ms (pre-mapped weights)
- Paged KV cache per job (no fragmentation)
- CUDA streams for parallel execution (different expert sets)
- INT4/INT8 compute kernels, speculative decoding (optional)

### 5. Marketplace
Decentralized Git-based catalog (no NPM/PyPI centralization).

**Features:**
- Expert registry: ~/.expert/registry.json (locally installed experts)
- Ed25519 signature verification per package
- Compatibility checking (base model hash, rope_scaling, incompatibilities)
- Discovery & search via metadata

### 6. Multi-Agent Orchestrator
Manages concurrent inference jobs with VRAM budgeting.

**Components:**
- FIFO queue with priority levels (0-9)
- VRAM budget checks before execution
- LRU eviction for hot experts
- Telemetry collection (latency, experts used, success rates)

## Data Flow Example

```
Prompt → Router (CPU, ~15-50ms) → Expert Selection 
  → Loader (hot: 1-10ms, cold: 50-200ms) 
  → Inference Runtime (GPU, 500ms-10s depending on seq_len)
  → Post-process (validation, metrics) → Output
```

**Performance Bottlenecks:**
- Router: Embedding computation + ANN search
- Loader: SSD I/O (cold cache)
- Inference: GPU sequence length, batch size, expert complexity
