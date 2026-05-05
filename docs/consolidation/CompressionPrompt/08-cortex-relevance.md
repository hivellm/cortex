# CompressionPrompt — Cortex Relevance

## Alignment with Cortex Architecture

### Cortex's Compression Need

**Current State**: Cortex consolidates/retrieves large contexts and routes them to LLMs (Expert, API).

**Pain Point**: Large consolidated findings or retrieved RAG chunks consume many tokens, increasing LLM API costs.

**Solution**: CompressionPrompt fits upstream of LLM calls, reducing token spend by 50% while maintaining 89%+ quality.

### Natural Integration Points

#### 1. cortex-consolidator Worker (Post-Consolidation)

```
Retriever → Consolidator → [COMPRESS HERE] → Expert/API
```

**Flow**:
1. Consolidator aggregates findings
2. CompressionPrompt compresses consolidated text
3. Dispatch to Expert with compact context

**Benefit**: 50% cost reduction on consolidated context size.

#### 2. cortex-compressor Worker (New Microservice)

Dedicated worker for compression-heavy workloads:

```
Request → [Compression Worker] → Cached Result
              ↓
         (stat filtering + quality check)
```

**Benefit**: Reusable across consolidator, router, and external APIs.

#### 3. cortex-embedder (Pre-Vectorization)

```
Text → [COMPRESS] → Vectorizer → Vector DB
```

**Flow**:
1. Compress text before sending to Vectorizer
2. Reduce embedding API tokens (50% savings)
3. Maintain semantic quality (91% preservation)

**Benefit**: Directly reduces Vectorizer API costs; no quality loss for embedding.

#### 4. cortex-router (Topic Card Routing)

```
Consolidated Findings → [COMPRESS] → Expert Router → Topic Cards
```

**Benefit**: Reduce context size for Expert routing decisions without losing information.

#### 5. cortex-api (External Compression Endpoint)

```
POST /compress
{ "text": "...", "compression_level": "balanced" }
→ { "compressed": "...", "savings": 50.2, "quality": 89.1 }
```

**Benefit**: External clients (CLI, Python, Node.js) can request compression.

## Synergy with Existing Cortex Components

### With cortex-core (Base Types)

CompressionPrompt would add:
- `CompressedContext` variant to context enums
- Quality metrics to response metadata
- Compression metadata (original size, compressed size, savings %)

### With cortex-storage (Persistence)

Compressed contexts could be cached:
- Store original + compressed (trade space for speed)
- Cache key: `hash(original) + compression_method`
- Reuse across similar consolidations

### With cortex-cli (Command-Line)

```bash
cortex compress <file> --level balanced --output <out>
cortex compress --stdin --stats  # Show savings %
```

### With Nexus (External Nodes)

Compress node descriptions before external ingestion:
```rust
let node = external_node;
let compressed_desc = compressor.compress(&node.description);
nexus.update_node(node.id, &compressed_desc).await?;
```

### With Vectorizer (Embedding)

Compress before embedding:
```rust
let text = "...";
let compressed = compressor.compress(&text);  // 50% shorter
let vector = vectorizer.embed(&compressed).await?;  // 50% cheaper
```

### With Expert (Agent Framework)

Compress inter-agent messages:
```rust
let message = expert_output;
let compressed = compressor.compress(&message);
next_expert.receive_context(&compressed)?;
```

## Planned cortex-compressor Worker

### Specification

**Purpose**: Standalone compression microservice for Cortex pipeline.

**Responsibilities**:
1. Receive compression requests (text, config)
2. Apply statistical filtering
3. Calculate quality metrics
4. Return compressed + metadata

**Interface**:
```rust
struct CompressionRequest {
    text: String,
    compression_level: CompressionLevel,  // Aggressive|Balanced|Conservative
    calculate_metrics: bool,
}

struct CompressionResponse {
    original: String,
    compressed: String,
    original_tokens: usize,
    compressed_tokens: usize,
    savings_pct: f32,
    quality: Option<QualityMetrics>,
}
```

**Configuration**:
```rust
enum CompressionLevel {
    Aggressive => { compression_ratio: 0.3, quality: 71% },
    Balanced => { compression_ratio: 0.5, quality: 89% },    // Default
    Conservative => { compression_ratio: 0.7, quality: 96% },
}
```

**Task Structure** (if implemented as Rulebook task):
```
phase11x_cortex-compressor-worker/
├── proposal.md        # Why: reduce LLM costs via compression
├── tasks.md           # 1. Core worker 2. Integration tests 3. Benchmarks
├── design.md          # Architecture, config
└── specs/
    └── compressor/spec.md  # Requirements: Shall compress with <5ms latency
```

## Cost Savings for Cortex

### Annual Estimate (Example)

**Assumptions**:
- Cortex processes 100M tokens/month to LLMs
- Avg cost: $5 per 1M tokens (mix of models)
- Compression: 50% (131 tokens saved per 262 tokens)

**Without CompressionPrompt**:
- Annual cost: 100M × 12 × $0.005 = **$6,000/year**

**With CompressionPrompt (50% reduction)**:
- Annual cost: 50M × 12 × $0.005 = **$3,000/year**
- **Savings: $3,000/year**

**Scaling**:
- 1B tokens/month → **$30,000/year saved**
- 10B tokens/month → **$300,000/year saved**

### Quality Trade-Off

- **Cost savings**: 50%
- **Quality retention**: 89%
- **ROI**: Excellent (lose 11% quality, save 50% cost)

## Challenges & Mitigation

### Challenge 1: Tokenizer Mismatch

**Issue**: CompressionPrompt uses MockTokenizer; Cortex may need Claude/GPT tokenizer.

**Mitigation**:
1. Implement Claude tokenizer wrapper (via external Anthropic tokenization API)
2. Implement GPT-4 tokenizer (via `tiktoken-rs`)
3. Use MockTokenizer as fallback (conservative, word-based)

### Challenge 2: Domain-Specific Terms

**Issue**: Cortex may have unique domain terms not in default `domain_terms` list.

**Mitigation**:
- Make `domain_terms` configurable per worker
- Expose via env var or config file: `CORTEX_COMPRESSION_DOMAIN_TERMS="Vectorizer,Synap,UMICP"`
- Allow users to customize in API requests

### Challenge 3: Quality Variability

**Issue**: Different consolidated contexts might compress differently (some loss more quality than others).

**Mitigation**:
- Always calculate and return `QualityMetrics`
- Log quality scores per consolidation
- Alert if quality drops below threshold (80%)
- Allow fallback to original if quality too low

### Challenge 4: Latency in Real-Time Paths

**Issue**: If Cortex routing is latency-critical, compression overhead might be problematic.

**Mitigation**:
- CompressionPrompt is <1ms (negligible compared to LLM call)
- Cache compressed results (same consolidation → same compression)
- Async compression if needed (fire-and-forget)

## Recommended Integration Roadmap

### Phase 1: Evaluation (Week 1)

- [ ] Benchmark CompressionPrompt on actual Cortex consolidated findings
- [ ] A/B test compression quality on Cortex use cases
- [ ] Measure latency impact on routing pipeline

### Phase 2: Proof of Concept (Week 2–3)

- [ ] Implement basic compression in cortex-consolidator
- [ ] Measure cost savings (token count before/after)
- [ ] Gather user feedback

### Phase 3: Production Integration (Week 4+)

- [ ] Create standalone cortex-compressor worker
- [ ] Add compression endpoint to cortex-api
- [ ] Update docs, examples
- [ ] Deploy to staging, then production

### Phase 4: Optimization (Ongoing)

- [ ] Implement tokenizer-specific variants
- [ ] Cache compression results
- [ ] Monitor quality metrics over time
- [ ] Adjust compression_ratio based on feedback

## Success Criteria for Cortex Integration

- [x] Compression algorithm is stable (statistical filtering)
- [x] Quality validated on real data (arXiv papers, LLM testing)
- [ ] Integration latency <5ms per request
- [ ] Cost savings measurable (50% token reduction)
- [ ] No degradation to user-facing quality
- [ ] Operational complexity minimal (no external services)
- [ ] Configuration flexible (allow users to tune)
- [ ] Monitoring in place (latency, quality, savings tracking)
