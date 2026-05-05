# CompressionPrompt — Operational

## Deployment

### Rust Crate (Primary)

**Package**: `compression-prompt v0.1.2`  
**Distribution**: Cargo registry (crates.io)  
**Minimum Rust**: 1.85+

**Installation**:
```toml
[dependencies]
compression-prompt = "0.1.2"

# Optional image feature
compression-prompt = { version = "0.1.2", features = ["image"] }
```

### Build & Compilation

**Release Build** (optimized):
```bash
cd rust && cargo build --release
```

**Profile Settings** (Cargo.toml):
```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

**Result**: Single-threaded binary ~5–10MB, no external runtime.

### Docker Deployment (Planned)

Dockerfile would provide:
- Rust 1.85+ base
- Compiled binary
- Reproducible environment

**Status**: Not yet provided; standard Rust Docker image can be used.

## Performance Characteristics

### Throughput

- **Actual**: 10.58 MB/s (measured on 1.6M token dataset)
- **Per-word**: ~0.5 microseconds
- **Latency**: <1ms for typical prompts (<10KB)

### Memory Usage

- **Peak**: ~5× input size (during tokenization + scoring)
- **Input**: 1.6M tokens (1.6MB text) → ~8MB peak memory
- **Stable**: Output size = ~50% of input

### CPU Usage

- **Single-threaded**: O(n + w log w) where n = text length, w = word count
- **Typical CPU**: <1ms on modern CPU (Intel i7, M1+)
- **Parallelization**: Potential via rayon (current impl is single-threaded)

## Logging & Observability

**Current**: No built-in logging.

**Observability Pattern** (for integration):
```rust
let start = Instant::now();
let compressed = filter.compress(&text, &tokenizer);
let duration = start.elapsed();
let ratio = compressed.len() as f32 / text.len() as f32;

println!("Compressed in {:.2}ms, ratio: {:.1}%", 
    duration.as_secs_f32() * 1000.0, 
    ratio * 100.0);
```

## Testing & Quality Assurance

### Unit Tests

**Coverage**: 41 test cases (as of v0.1.2)

**Areas**:
- Word scoring logic
- Protected span detection (JSON, code, paths)
- Edge cases (empty text, very short, all stopwords)
- Unicode handling
- Configuration validation

**Run**: `cargo test --release`

### Integration Tests

**Coverage**: 7 integration tests

**Areas**:
- End-to-end compression
- Quality metrics calculation
- Performance benchmarks
- Real data (sample arXiv papers)

**Run**: `cargo test --release -- --test-threads=1`

### Validation Tests (Real Data)

**Dataset**: 200 arXiv papers (1.6M tokens)

**Metrics**:
- Compression ratio: 50.0%
- Quality score: 88.6%
- Keyword retention: 100.0%
- Entity retention: 91.8%

**LLM A/B Testing**: 350+ test pairs across 6 models

**Run**: `cargo run --release --bin test_statistical`

## Configuration & Environment

### Feature Flags

```toml
[features]
default = ["statistical"]
statistical = []              # Core algorithm (always on)
image = ["dep:image", "dep:imageproc", "dep:ab_glyph"]  # Optional: PNG/JPEG rendering
full = ["statistical", "image"]
```

### Runtime Configuration

No environment variables required. All configuration via Rust struct:

```rust
let config = StatisticalFilterConfig {
    compression_ratio: 0.5,
    idf_weight: 0.3,
    // ... other fields
    domain_terms: vec!["Vectorizer", "Synap"].iter().map(|s| s.to_string()).collect(),
};
```

## Monitoring & Metrics

### Key Metrics for Cortex Integration

| Metric | Target | Tool |
|--------|--------|------|
| Compression latency | <5ms | `Instant::now()` |
| Quality score | >85% | `QualityMetrics::calculate()` |
| Token savings | >40% | Token count comparison |
| Memory peak | <100MB | OS monitoring |
| Error rate | 0% | Test suite |

### Alerting (Recommended)

For Cortex worker integration:
- Alert if compression latency > 50ms (indicates input size explosion or bug)
- Alert if quality drops below 80% (indicates algorithm drift or configuration issue)
- Track token savings % per day (monitor effectiveness)

## Troubleshooting

### Compression Too Slow

**Symptoms**: Latency > 100ms for typical inputs  
**Causes**:
1. Input text unusually large (>10MB) — expected
2. Tokenizer implementation slow — use MockTokenizer for baseline
3. Excessive scoring overhead — check `enable_protection_masks`, `enable_contextual_stopwords` flags

**Fix**: Profile with `cargo flamegraph` or reduce input size for testing.

### Quality Dropping

**Symptoms**: Overall score < 80%  
**Causes**:
1. Compression ratio too aggressive (<0.3) — increase to 0.5–0.7
2. Domain terms not configured — add relevant terms to `domain_terms`
3. Protected spans not working — verify JSON/code blocks are detected

**Fix**: Adjust `compression_ratio` or add domain-specific terms.

### Corrupted Output

**Symptoms**: JSON/code broken, identifiers mangled  
**Causes**:
1. Old version (<0.1.2) without protection masks
2. Domain terms list incomplete

**Fix**: Update to v0.1.2 and configure `domain_terms`.

## Backward Compatibility

### Version History

- **v0.1.2** (current): Protected spans (JSON, code), 41 tests
- **v0.1.1**: Core statistical filtering
- **v0.1.0**: Initial release with dictionary compression

### Breaking Changes

None (additive only). v0.1.2 is fully backward compatible with v0.1.1.

### Upgrade Path

No action required. Dependencies on `compression-prompt` will automatically use latest patch version.

## Performance Tuning

### For Maximum Throughput

```rust
let config = StatisticalFilterConfig::default();
// Already optimized (single-threaded, linear complexity)
// Further gains require algorithmic change (e.g., approximate IDF)
```

### For Maximum Quality

```rust
let config = StatisticalFilterConfig {
    compression_ratio: 0.7,  // Keep 70%
    ..Default::default()
};
// Trades 30% token savings for 96%+ quality
```

### For Specific Domains

```rust
let config = StatisticalFilterConfig {
    idf_weight: 0.4,         // Prioritize rare terms
    pos_weight: 0.3,         // Prioritize verbs/nouns
    domain_terms: vec!["Vectorizer", "Synap", "MyTerm"].iter().map(|s| s.to_string()).collect(),
    ..Default::default()
};
```

## Production Readiness Checklist

- [x] Stable algorithm (statistical filtering, proven 50% compression)
- [x] Protected data (JSON, code, structured data preserved)
- [x] Unit tests (41 tests, 100% pass)
- [x] Integration tests (7 tests)
- [x] Real-world validation (200 papers, 1.6M tokens)
- [x] Multi-LLM A/B testing (350+ pairs, 6 models)
- [x] Performance profiling (10.58 MB/s throughput)
- [x] No external dependencies (model-free)
- [x] Thread-safe (Send + Sync implementations)
- [x] Error handling (graceful degradation)
- [ ] Docker image (future: standardized deployment)
- [ ] Logging/observability (future: structured logs)
- [ ] Multi-language SDKs (future: Python, TypeScript)

**Status**: Production-ready for Rust integrations; external SDKs pending.
