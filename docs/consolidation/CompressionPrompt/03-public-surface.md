# CompressionPrompt — Public APIs & Surface

## Rust Crate API

### Primary Entry Point

```rust
use compression_prompt::statistical_filter::{StatisticalFilter, StatisticalFilterConfig};
use compression_prompt::tokenizer::Tokenizer;

let config = StatisticalFilterConfig::default();
let filter = StatisticalFilter::new(config);
let compressed = filter.compress(&text, &tokenizer);
```

### Key Methods

#### StatisticalFilter

```rust
pub fn new(config: StatisticalFilterConfig) -> Self
pub fn compress(&self, text: &str, tokenizer: &dyn Tokenizer) -> String
pub fn score_tokens(&self, text: &str, tokenizer: &dyn Tokenizer) -> Vec<TokenImportance>
```

#### StatisticalFilterConfig

```rust
pub struct StatisticalFilterConfig {
    pub compression_ratio: f32,
    pub idf_weight: f32,
    pub position_weight: f32,
    pub pos_weight: f32,
    pub entity_weight: f32,
    pub entropy_weight: f32,
    pub enable_protection_masks: bool,
    pub enable_contextual_stopwords: bool,
    pub preserve_negations: bool,
    pub preserve_comparators: bool,
    pub domain_terms: Vec<String>,
    pub min_gap_between_critical: usize,
}

impl Default for StatisticalFilterConfig {
    // compression_ratio: 0.5
    // Weights: 0.3, 0.2, 0.2, 0.2, 0.1 (IDF, pos, POS, entity, entropy)
}
```

#### QualityMetrics

```rust
pub struct QualityMetrics {
    pub keyword_retention: f32,
    pub entity_retention: f32,
    pub vocabulary_ratio: f32,
    pub information_density: f32,
    pub overall_score: f32,
}

impl QualityMetrics {
    pub fn calculate(original: &str, compressed: &str) -> Self
}
```

#### Tokenizer Trait

```rust
pub trait Tokenizer: Send + Sync {
    fn encode(&self, text: &str) -> Vec<Token>;
    fn decode(&self, tokens: &[Token]) -> String;
    fn count_tokens(&self, text: &str) -> usize;
    fn name(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token(pub u32);
```

## Feature Flags

| Flag | Enables | Default |
|------|---------|---------|
| `statistical` | Core statistical filtering | Yes |
| `image` | Image rendering (PNG/JPEG) | No |
| `full` | All features | No |

### Usage

```toml
[dependencies]
compression-prompt = { version = "0.1.2", features = ["full"] }
```

## Cargo Binaries (CLI)

### Available Binaries

- `test_statistical` — Compress full dataset (200 papers)
- `bench_quality` — Quality metrics on 20 papers
- `generate_llm_dataset` — Create 63 prompt pairs for LLM testing
- Paper-to-image converters (beta feature)

### Running Binaries

```bash
cd rust && cargo build --release

cargo run --release --bin test_statistical
cargo run --release --bin bench_quality
cargo run --release --bin generate_llm_dataset
```

## Configuration Presets

### Default (Balanced)

```rust
StatisticalFilterConfig::default()
// compression_ratio: 0.5
// Quality: 89%, Savings: 50%
// Use case: General production
```

### Conservative (High Precision)

```rust
StatisticalFilterConfig {
    compression_ratio: 0.7,
    ..Default::default()
}
// Quality: 96%, Savings: 30%
// Use case: Technical, legal, medical
```

### Aggressive (Maximum Savings)

```rust
StatisticalFilterConfig {
    compression_ratio: 0.3,
    ..Default::default()
}
// Quality: 71%, Savings: 70%
// Use case: Triaging, filtering
```

## Integration Pattern

```rust
// Basic
let compressed = filter.compress(&text, &tokenizer);

// With quality check
let metrics = QualityMetrics::calculate(&text, &compressed);
if metrics.overall_score > 0.85 {
    send_to_llm(compressed);
} else {
    send_to_llm(text); // Fallback
}

// With cost tracking
let original_tokens = tokenizer.count_tokens(&text);
let compressed_tokens = tokenizer.count_tokens(&compressed);
let cost_savings = (original_tokens - compressed_tokens) as f32 * 0.000005;
```

## Error Handling

CompressionPrompt uses graceful degradation:
- Empty text → Returns empty string
- Very short text (<100 words) → Returns original
- All stop words → Keeps minimum viable content
- Unicode → Handled correctly (no panics)
- Protected spans (JSON, code) → 100% preserved

No explicit error type; compression always succeeds (worst case: returns original text).
