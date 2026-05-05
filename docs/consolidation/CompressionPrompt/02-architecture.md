# CompressionPrompt — Architecture

## High-Level Design

```
Input Text
    ↓
[Detect Protected Spans] ← code blocks, JSON, identifiers
    ↓
[Split into Words]
    ↓
[Score Each Word] ← IDF, position, POS, entities, entropy
    ↓
[Select Top N%] ← based on compression_ratio
    ↓
[Fill Gaps] ← re-add tokens between critical terms
    ↓
[Reconstruct Text] ← preserve original word order
    ↓
Output (50% smaller)
```

## Core Components

### 1. Statistical Filter (`statistical_filter.rs`)

Main compression engine. Implements five-factor word importance scoring:

- **IDF (30%)**: Rare words scored higher (domain terms preserved)
- **Position (20%)**: Start/end-of-text words prioritized
- **POS (20%)**: Content words over function words (with contextual exceptions)
- **Entities (20%)**: Named entities, numbers, domain terms
- **Entropy (10%)**: Vocabulary diversity maintenance

**Key Features**:
- Protection masks (JSON, code blocks, paths, identifiers)
- Contextual stopword preservation ("how to", "in src/")
- Critical term handling (negations, comparators, domain terms)
- Gap filling between widely-separated critical tokens

**Configuration**:
```rust
StatisticalFilterConfig {
    compression_ratio: f32,              // 0.3–0.7 (default: 0.5)
    idf_weight: f32,                     // (default: 0.3)
    position_weight: f32,                // (default: 0.2)
    pos_weight: f32,                     // (default: 0.2)
    entity_weight: f32,                  // (default: 0.2)
    entropy_weight: f32,                 // (default: 0.1)
    enable_protection_masks: bool,       // (default: true)
    enable_contextual_stopwords: bool,   // (default: true)
    preserve_negations: bool,            // (default: true)
    domain_terms: Vec<String>,           // ["Vectorizer", "Synap", "UMICP", ...]
}
```

### 2. Quality Metrics (`quality_metrics.rs`)

Objective measurement of compression impact:
- **Keyword Retention**: % of important terms preserved
- **Entity Retention**: % of named entities preserved
- **Vocabulary Ratio**: Diversity maintained
- **Information Density**: Unique words / total words
- **Overall Score**: Weighted combination (typically 88–92%)

### 3. Tokenizer Interface (`tokenizer.rs`)

Pluggable abstraction for different LLM tokenizers.

**Trait**:
```rust
pub trait Tokenizer: Send + Sync {
    fn encode(&self, text: &str) -> Vec<Token>;
    fn decode(&self, tokens: &[Token]) -> String;
    fn count_tokens(&self, text: &str) -> usize;
    fn name(&self) -> &str;
}
```

**Current Implementations**:
- `MockTokenizer`: Whitespace-based (development/testing)
- Future: Claude, GPT-4, Gemini, Mistral (via external crates)

## Data Flow

```
Word: "the"          │  Word: "Bayesian"
├─ IDF: 0.1 (common) │  ├─ IDF: 0.95 (rare)
├─ Position: 0.5     │  ├─ Position: 0.8 (start)
├─ POS: 0.1 (stop)   │  ├─ POS: 1.0 (important)
├─ Entity: 0.0       │  ├─ Entity: 0.3
├─ Entropy: 0.3      │  ├─ Entropy: 0.7
└─ Score: 0.18 (LOW) │  └─ Score: 0.775 (HIGH)
   [REMOVE]          │     [KEEP]
```

## Performance Characteristics

| Metric | Value |
|--------|-------|
| Time Complexity | O(n + w log w) where n = text length, w = word count |
| Space Complexity | O(w) for word storage, scores, output |
| Actual Throughput | 10.58 MB/s (tested on 1.6M tokens) |
| Peak Memory | ~50MB (for 1.6M token input) |
| Per-Word Cost | ~0.5μs |

## Algorithm Properties

- **Deterministic**: Same input always produces same output
- **Linear scaling**: Performance grows proportionally with input size
- **No external calls**: Model-free, zero network/API dependencies
- **Stateless**: Each compression independent, no persistent state
- **Unicode-aware**: Handles multi-byte characters correctly

## Deprecated Legacy: Dictionary Compression

Older approach using n-gram dictionaries:
- Only 6% compression (vs. 50% for statistical)
- 42x slower (38s vs. 0.92s for 1.6M tokens)
- 15% success rate (requires highly repetitive text)
- Status: Kept for backward compatibility, not recommended

**Files**: `dictionary.rs`, `compressor.rs`, `ngram.rs`, `marker.rs`
