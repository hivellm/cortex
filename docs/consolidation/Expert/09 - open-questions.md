# Expert System — Open Questions & Gaps

## Implementation Roadmap Gaps

### 1. Multi-Expert Composition Semantics
**Status:** Partially designed, not implemented

**Questions:**
- When 3+ experts are loaded, how to handle conflicting outputs?
  - Example: expert-json-parser wants strict validation, expert-generalist wants lenient parsing
- Composition order matters: should it be heuristic (order in keywords), or learned?
- Can conflicts be detected at composition time?

**Impact on Cortex:** Need defined behavior before Cortex can reliably use multi-expert inferences

### 2. Expert Selection Quality Metrics
**Status:** Telemetry collected, but no automated ranking

**Open:**
- How to benchmark "accuracy" across diverse expert domains (JSON vs. Cypher vs. SQL)?
- Should Cortex define domain-specific accuracy metrics (F1, BLEU, schema validation)?
- How often to retrain router on success/failure feedback?

### 3. Marketplace Indexing
**Status:** Git-based, manual discovery

**Open:**
- How do users discover experts without centralized catalog?
- Should Vectorizer index expert descriptions for semantic search?
- Is GitHub search + stars sufficient, or need custom index?

**Impact on Cortex:** Cortex might need to maintain local expert inventory

### 4. KV Cache Invalidation Strategy
**Status:** Per-job isolation designed, invalidation rules unclear

**Open:**
- If user swaps experts mid-generation, should KV cache be flushed?
- Can we reuse KV across expert sets if composition is compatible?
- Performance tradeoff: keep cache (might be incorrect) vs. flush (safe but slow)?

### 5. Router Latency Optimization
**Status:** Heuristics + embeddings + mini-policy outlined, not tuned

**Open:**
- Can router decisions be cached/memoized aggressively?
- Is 15-50ms acceptable, or must it drop to <5ms?
- Should Cortex pre-compute expert plans for common task types?

### 6. Expert Dependency Resolution
**Status:** Linear expert loading (order matters), no dependency graph

**Open:**
- Can expert A declare "requires expert B"? (e.g., JSON parser needs English understanding)
- How to handle circular or conflicting dependencies?
- Auto-install dependencies, or error loudly?

### 7. Quantization Strategy Post-Training
**Status:** INT4 chosen for base, expert weights not quantized

**Open:**
- Should expert LoRA weights be quantized (INT8/NF4)?
- Tradeoff: file size (15 MB → 10 MB) vs. quality loss
- Should quantization be configurable per expert?

**Impact on Cortex:** Affects VRAM budget calculations, inference speed

### 8. Expert Versioning & Compatibility
**Status:** Manifest versioning exists, compatibility matrix vague

**Open:**
- What breaks backward compatibility? (base model, quantization, rope_scaling)
- Should old expert versions be kept installable, or deprecated?
- How to handle expert "drift" (model updates that change outputs)?

### 9. Synthetic Data Quality Control
**Status:** Filtering rules implemented (diversity, validation), no human review

**Open:**
- Should Cortex review/validate expert training data before trusting results?
- Can Cortex detect when expert predictions are hallucinations vs. valid?
- How often to regenerate training data as base models improve?

### 10. Multi-GPU Scaling
**Status:** Single-GPU (CUDA device:0) assumed

**Open:**
- Can base model be split across GPUs (tensor parallelism)?
- Can experts be distributed (expert A on GPU0, B on GPU1)?
- Is orchestrator responsible for load balancing?

**Impact on Cortex:** Affects deployment scalability for high-throughput scenarios

## Known Limitations

1. **No expert merging:** Can't optimize "always-load JSON+English" into single artifact
2. **Router heuristics hardcoded:** Regex patterns not domain-learnable yet
3. **No feedback loop:** Router doesn't auto-update based on expert success/failure
4. **Marketplace unsigned:** Git repos not cryptographically verified (manual trust)
5. **No auto-retraining:** Experts don't auto-update when base model changes
6. **Context limits:** 200k token ceiling due to YaRN; dynamic stretching not implemented
7. **No speculative decoding:** Base model alone doesn't draft (P6 feature)

## Blockers for Cortex Integration

1. **Router decision transparency:** Cortex needs audit trail of "why these experts?"
   - **Solution pending:** Export decision trace from router

2. **Telemetry aggregation:** How to bulk-export inference logs to Cortex?
   - **Solution pending:** Streaming API or batch export endpoint

3. **Expert catalog discovery:** How does Cortex know all available experts?
   - **Solution pending:** Index expert-registry.json + HF Hub via Vectorizer

4. **Output validation:** Cortex needs schema validation post-inference
   - **Solution pending:** Optional validation plugin per expert

5. **Feedback loop:** Expert router needs Cortex success/failure signals
   - **Solution pending:** Callback mechanism or polling for outcome updates
