# CompressionPrompt — Integrations

## HiveLLM Projects Using CompressionPrompt

### Vectorizer

**Status**: Planned integration
**Use Case**: Compress dense vector descriptions and metadata before indexing
**Integration Point**: Pre-processing pipeline (optional)

### Nexus (Graph Database)

**Status**: Planned integration
**Use Case**: Compress node/edge descriptions for storage optimization
**Integration Point**: Write pipeline before persistence

### Synap (Multi-Modal Search)

**Status**: Planned integration
**Use Case**: Compress text metadata accompanying multi-modal embeddings
**Integration Point**: Content enrichment pipeline

### Expert (Agent Framework)

**Status**: Planned integration
**Use Case**: Compress long tool outputs and context between agent calls
**Integration Point**: Inter-agent communication layer

### Cortex (This Project)

**Status**: Primary consumer
**Use Case**: Compress consolidated/retrieved context before LLM routing
**Planned Integration Points**:
1. **cortex-consolidator worker** — Compress consolidated findings before dispatch
2. **cortex-compressor worker** (new) — Dedicated compression microservice
3. **cortex-embedder** — Compress text before Vectorizer ingestion
4. **cortex-router** — Compress context before topic card routing to Expert
5. **cortex-api** — Compression endpoint for external clients

## Dependencies & Tokenizer Integration

### Current Dependencies

```toml
[dependencies]
serde = "1.0"
serde_json = "1.0"
thiserror = "2.0"
anyhow = "1.0"
regex = "1.10"
ahash = "0.8"
rayon = "1.10"
unicode-segmentation = "1.12"
tiktoken-rs = "0.7.0"
chrono = "0.4"
```

**Notable**: `tiktoken-rs` provides tokenizer support but is currently unused; actual tokenizers are external.

### Planned Tokenizer Integrations

| LLM | Library | Status | Notes |
|-----|---------|--------|-------|
| Claude | Custom/external | Pending | Match Anthropic's tokenization |
| GPT-4 | `tiktoken-rs` | Pending | cl100k_base vocab |
| Mistral | `tokenizers` crate | Pending | SentencePiece-based |
| Gemini | Custom/external | Pending | Requires Google vocab |

### Vectorizer SDK Integration

**If Cortex uses Vectorizer for embedding compressed text**:

```rust
// Hypothetical integration
use vectorizer_sdk::client::VectorizerClient;
use compression_prompt::statistical_filter::StatisticalFilter;

let filter = StatisticalFilter::default();
let vectorizer = VectorizerClient::new(config);

let text = "... long document ...";
let compressed = filter.compress(&text, &tokenizer);
let vector = vectorizer.embed(&compressed).await?;
```

**Benefits**:
- Reduce embedding API costs (50% fewer tokens)
- Faster embedding generation
- Maintained semantic similarity (91% quality)

## Usage Patterns in Cortex

### Pattern 1: Pre-LLM Compression

```rust
// In cortex-api route handler
let consolidated = retriever.get_consolidated_findings();
let compressed = compressor.compress(&consolidated)?;
let response = llm.call(compressed).await?;
```

### Pattern 2: Worker-Based Compression

```rust
// In cortex-consolidator worker
struct ConsolidatorWorker {
    consolidator: Consolidator,
    compressor: StatisticalFilter,
}

impl ConsolidatorWorker {
    fn process(&self, input: WorkerInput) -> Result<WorkerOutput> {
        let consolidated = self.consolidator.consolidate(&input)?;
        let compressed = self.compressor.compress(&consolidated, &self.tokenizer)?;
        Ok(WorkerOutput { compressed })
    }
}
```

### Pattern 3: Optional Conditional Compression

```rust
// Compress only if text exceeds threshold
if text.split_whitespace().count() > 1000 {
    compressor.compress(&text)
} else {
    text.to_string()
}

// Or: Compress, but fallback if quality drops below threshold
let compressed = compressor.compress(&text);
let metrics = QualityMetrics::calculate(&text, &compressed);
if metrics.overall_score > 0.85 {
    compressed
} else {
    text
}
```

### Pattern 4: Configurable Compression Levels

```rust
// Allow users to trade off quality vs. cost
match request.compression_level {
    CompressionLevel::None => text,
    CompressionLevel::Aggressive => 
        StatisticalFilterConfig { compression_ratio: 0.3, ..Default::default() },
    CompressionLevel::Balanced => 
        StatisticalFilterConfig::default(), // 0.5
    CompressionLevel::Conservative => 
        StatisticalFilterConfig { compression_ratio: 0.7, ..Default::default() },
}
```

## Performance Characteristics for Integration

### Latency Budget

| Scenario | Max Latency | Typical |
|----------|------------|---------|
| RAG context pre-compression | 10ms | <1ms |
| Worker-integrated compression | 100ms | <10ms |
| Batch API compression | 5s | <1s |
| Real-time API request | 50ms | <5ms |

### Memory Budget

| Input Size | Peak Memory | Notes |
|------------|------------|-------|
| 1MB | ~5MB | 5× input (tokenization + scoring + output) |
| 10MB | ~50MB | Linear scaling |
| 100MB | ~500MB | Single-thread friendly |

**Thread Safety**: All components are `Send + Sync`, suitable for concurrent workers.

## Migration Path from External Compression

If Cortex currently uses external compression (e.g., Python LLMLingua):

1. **Phase 1**: Measure baseline (token savings, quality, latency)
2. **Phase 2**: Integrate CompressionPrompt in parallel
3. **Phase 3**: A/B test results (Cortex workers × 10 requests each)
4. **Phase 4**: Migrate fully (replace external calls)
5. **Phase 5**: Remove external dependency, reduce ops burden

**Expected Improvements**:
- Latency: 100–1000× faster (external API → local function)
- Cost: Eliminate external service tier
- Reliability: No network dependency
- Quality: 89–93% (comparable to LLMLingua + faster)
