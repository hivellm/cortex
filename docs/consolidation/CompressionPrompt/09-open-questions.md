# CompressionPrompt — Open Questions

## Tokenizer Integration

### Q1: Which Tokenizers to Support First?

**Current State**: MockTokenizer (whitespace-based) works for development.

**Issue**: Real LLMs use different tokenizers (Claude, GPT-4, Mistral, Gemini).

**Options**:
1. **Option A**: Use MockTokenizer for all (conservative, safe, but loses precision)
2. **Option B**: Integrate Claude tokenizer via Anthropic's public API
3. **Option C**: Use `tiktoken-rs` for GPT-4, implement others later
4. **Option D**: Support multiple tokenizers, let users choose

**Recommendation**: Option D (multiple tokenizers).

**Next Steps**:
- Poll Cortex users: Which tokenizers matter most? (Claude, GPT-4, both?)
- Investigate Anthropic SDK for tokenization capabilities
- Evaluate `tiktoken-rs` maturity and accuracy

### Q2: Should CompressionPrompt Ship Real Tokenizer Implementations?

**Current State**: CompressionPrompt repo doesn't include real tokenizers; trait is pluggable.

**Issue**: Users need tokenizers but may not know how to implement them.

**Options**:
1. Ship with mock only, users implement their own
2. Provide example implementations (Claude, GPT-4) in docs/examples
3. Create separate `compression-prompt-tokenizers` crate with real implementations
4. Use external crates (tiktoken-rs, etc.) as optional dependencies

**Recommendation**: Option 3 (separate crate) to keep CompressionPrompt lean.

**Next Steps**: Design tokenizers crate if Cortex needs this.

---

## Real-World Integration

### Q3: How Does Compression Affect Multi-Turn Conversations?

**Current State**: Compression designed for single-turn compression.

**Issue**: Cortex may process multi-turn conversations (human → Expert → LLM → response → next turn).

**Question**: Should each turn be compressed independently, or the entire conversation?

**Options**:
1. Compress each turn separately (simpler, faster)
2. Compress entire conversation together (better context preservation, but loses turn boundaries)
3. Compress only LLM responses, preserve human queries
4. Make it configurable

**Recommendation**: Option 3 (compress LLM responses only, preserve queries).

**Rationale**: Human queries are typically short; LLM responses are long and full of filler.

**Next Steps**: Test multi-turn A/B tests; measure quality impact.

### Q4: Should Compression Cache Results for Identical Inputs?

**Current State**: No caching; each `compress()` call recomputes.

**Issue**: Cortex may compress the same consolidation multiple times (e.g., multiple Expert calls).

**Options**:
1. No caching (stateless, simple, but wasteful)
2. Optional LRU cache in `StatisticalFilter`
3. External caching layer (user's responsibility)
4. Built-in memoization with TTL

**Recommendation**: Option 2 (optional LRU cache).

**Next Steps**: Design cache interface; measure cache hit rates on Cortex workloads.

---

## Quality & Validation

### Q5: How to Validate Quality for Domain-Specific Content?

**Current State**: Validated on arXiv papers (academic ML content).

**Issue**: Cortex may compress other content (logs, configs, API responses, code).

**Question**: Are validation results (89% quality) valid for these domains?

**Options**:
1. Assume generalization holds (risky)
2. Run additional A/B tests on Cortex-specific content
3. Develop domain-specific quality metrics
4. Allow users to self-validate (provide tools, but no guarantees)

**Recommendation**: Option 2 (A/B test on Cortex workloads).

**Next Steps**:
- Collect sample consolidated findings from Cortex
- Run A/B tests with Expert on compressed vs. original
- Measure impact on Expert output quality

### Q6: What Happens to Rare/Specialized Domains?

**Current State**: Statistical filtering preserves rare words (high IDF score).

**Issue**: Some domains may have high-frequency important terms (e.g., medical, legal).

**Question**: Does compression work for specialized vocabularies?

**Example**: "The patient experienced myocardial infarction" — both "patient" and "infarction" are important, but "infarction" is rarer.

**Recommendation**: Add domain-specific weights.

**Next Steps**:
- Design domain config: `domain_importance_terms: vec!["myocardial", "infarction"]`
- Test on legal/medical corpora
- Document domain-specific tuning

---

## Operational

### Q7: How to Handle Compression Failures?

**Current State**: Compression never fails; returns original text as fallback.

**Issue**: Cortex may want explicit error handling (e.g., "compression failed, reject request").

**Question**: Should `compress()` return `Result<String, Error>` instead of `String`?

**Options**:
1. Keep current (always returns String, never fails)
2. Add optional error mode via config flag
3. Create strict version: `compress_strict() -> Result<String, Error>`
4. Log failures, but return original text

**Recommendation**: Option 4 (status quo, but add logging).

**Rationale**: Graceful degradation is safer for production. Logging allows debugging.

**Next Steps**: Add observability layer (logging, metrics).

### Q8: How to Version & Upgrade Compression Algorithms?

**Current State**: Single algorithm (statistical_50) is hardcoded.

**Issue**: Future versions may introduce new algorithms or improvements.

**Question**: How to handle backward compatibility and upgrades?

**Options**:
1. Always use latest algorithm (breaking, unsuitable for caching)
2. Versioned algorithms: `compress_v1()`, `compress_v2()` (complex)
3. Configurable algorithm field: `algorithm: CompressionAlgorithm` (future-proof)
4. Stable default, but allow opt-in to new algorithms

**Recommendation**: Option 3 (configurable algorithm).

**Design**:
```rust
pub enum CompressionAlgorithm {
    Statistical50,
    StatisticalAdaptive,  // Future
    Hybrid,               // Future
}

pub struct StatisticalFilterConfig {
    pub algorithm: CompressionAlgorithm,
    // ...
}
```

**Next Steps**: Plan for future algorithms; document versioning policy.

---

## Performance & Scaling

### Q9: Can Compression Scale to Cortex's Full Workload?

**Current State**: Tested on 1.6M tokens (200 papers).

**Issue**: Cortex may process 100M+ tokens/month.

**Question**: Does O(n + w log w) complexity hold at 100M scale?

**Concerns**:
1. Sorting step may become bottleneck
2. Memory usage scales linearly (potential OOM)
3. IDF calculation over full corpus (not implemented, assumes pre-computed)

**Recommendation**: Benchmarking on larger datasets.

**Next Steps**:
- Profile compression on 1B+ token corpus
- Optimize sorting (e.g., partial sort, quickselect)
- Implement streaming IDF calculation if needed

### Q10: Should Compression Be Parallelized?

**Current State**: Single-threaded.

**Issue**: Cortex workers are multi-threaded; could benefit from parallelization.

**Options**:
1. Keep single-threaded (simple, but underutilizes CPU)
2. Parallelize word scoring via rayon
3. Parallelize entire batch processing
4. Leave to application layer (users parallelize `compress()` calls)

**Recommendation**: Option 2 (parallelize word scoring).

**Design**:
```rust
let scores: Vec<f32> = words
    .par_iter()
    .map(|word| self.score_word(word))
    .collect();  // Uses rayon
```

**Benefit**: ~3–4× speedup on 4-core CPU.

**Next Steps**: Implement rayon parallelization; measure speedup.

---

## API & Ecosystem

### Q11: Should CompressionPrompt Provide Python/TypeScript Bindings?

**Current State**: Rust-only crate.

**Issue**: Cortex may have Python/Node.js components.

**Question**: Provide official bindings, or community implementations?

**Options**:
1. Official Python binding (via PyO3)
2. Official Node.js binding (via napi)
3. Community contributions only
4. WASM + JavaScript shim
5. HTTP microservice wrapper

**Recommendation**: Option 5 (HTTP wrapper) as MVP, then Python (PyO3) if demand.

**Next Steps**: Design HTTP API if Cortex needs non-Rust integration.

### Q12: Should CompressionPrompt Support Other Compression Algorithms?

**Current State**: Statistical filtering only.

**Issue**: Future research may find better algorithms (Gisting, LongLLaMA-Compressor, etc.).

**Question**: Evolve CompressionPrompt to support multiple algorithms, or stay focused?

**Options**:
1. Stay focused: Statistical filtering only
2. Plugin architecture: Allow algorithm implementations
3. Unified interface: Swap algorithms at config time
4. Create parent crate `prompt-compression` with multiple algos

**Recommendation**: Option 1 (stay focused) for now.

**Rationale**: One excellent algorithm > multiple mediocre ones. Revisit if new algos prove superior in production.

**Next Steps**: Document why statistical was chosen; plan future research.

---

## Documentation & Knowledge Transfer

### Q13: What Documentation is Missing?

**Current State**:
- README.md ✅
- Architecture.md ✅
- EXAMPLES.md ✅
- API docs ✅
- Benchmarks ✅

**Gaps**:
- Domain-specific tuning guide
- Troubleshooting guide
- Integration guide for Cortex
- Contributing guide
- Performance optimization guide

**Recommendation**: Create in this order:
1. Integration guide (Cortex-specific)
2. Troubleshooting guide
3. Contributing guide

**Next Steps**: Write these docs as Cortex integration begins.

### Q14: How to Handle Institutional Knowledge About Quality Trade-Offs?

**Current State**: Quality metrics exist, but no guidance on interpreting them.

**Issue**: Users may not understand when 89% quality is acceptable vs. when 96% is needed.

**Question**: How to document decision-making process?

**Recommendation**: Create decision matrix in docs.

**Example**:
```
Use Case              | Recommended Config | Quality | Savings | Why
RAG Context          | statistical_50     | 89%     | 50%     | Good balance
Expert Reasoning     | statistical_70     | 96%     | 30%     | Preserve nuance
Cost Optimization    | statistical_50     | 89%     | 50%     | Primary goal
Real-Time Triaging   | statistical_30     | 71%     | 70%     | Speed critical
```

**Next Steps**: Build decision tree; ship with crate.
