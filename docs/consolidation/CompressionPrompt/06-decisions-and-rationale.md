# CompressionPrompt — Decisions & Rationale

## Core Algorithm: Statistical Filtering Over Dictionary Compression

### Decision

Chose **statistical filtering** as the primary algorithm over **dictionary-based n-gram compression**.

### Rationale

| Criterion | Statistical | Dictionary | Winner |
|-----------|------------|------------|--------|
| Compression Ratio | 50% | 6% | Statistical |
| Speed | <1ms | 38s | Statistical |
| Success Rate | 100% | 15% | Statistical |
| Complexity | Simple scoring | Complex dict mgmt | Statistical |
| Reliability | Deterministic | Hit-or-miss | Statistical |

**Key Finding**: Dictionary compression requires highly repetitive text (narrow academic domain). Statistical filtering works universally.

**Cost**: 831,365 tokens saved across 200 papers. Dictionary would save only ~100K tokens.

### Implementation Status

- Statistical: Stable, production-ready (v0.1.2)
- Dictionary: Kept for backward compatibility, not recommended for new projects

---

## Five-Factor Scoring: IDF + Position + POS + Entity + Entropy

### Decision

Composite score combining five independent importance signals rather than single metric (e.g., IDF alone).

### Rationale

**Single IDF weakness**: Would remove domain-agnostic words (e.g., "learning", "model") that are contextually important in ML papers.

**Five-factor strength**:
- **IDF (30%)**: Captures rarity (technical terms)
- **Position (20%)**: Captures structural importance (abstract, introduction)
- **POS (20%)**: Captures word class (verbs > articles)
- **Entity (20%)**: Captures proper nouns, numbers (always important)
- **Entropy (10%)**: Captures vocabulary diversity (prevents redundancy collapse)

**Trade-off**: More computation (O(n) scoring) but better quality (91% vs. theoretical 70% with IDF alone).

---

## Compression Ratio Default: 0.5 (50% Compression)

### Decision

Default configuration keeps 50% of tokens (50% compression).

### Rationale

**Quality vs. Cost Pareto Frontier**:

| Ratio | Savings | Quality | LLM (Claude) | ROI |
|-------|---------|---------|--------------|-----|
| 0.3 | 70% | 71% | ⚠️ Risky | High |
| 0.5 | 50% | 89% | ✅ Excellent | **BEST** |
| 0.7 | 30% | 96% | ✅ Perfect | Good |

**0.5 is the inflection point**: Quadrants-quality trade-off (89% quality for 50% savings) outperforms both extremes.

**Supporting Data**: Grok-4 achieves 93% quality at 50% savings; Claude Sonnet achieves 91%.

---

## Protection Masks for JSON, Code, Structured Data (v0.1.2)

### Decision

Automatically detect and preserve JSON objects, code blocks, file paths, identifiers, and domain terms at 100%.

### Rationale

**Problem (v0.1.1)**: Dictionary compression would corrupt JSON:
```
Before: {"user": {"name": "Alice", "age": 30}}
After:  {"user": {"name": "Alice" 30}}  ← Broken!
```

**Solution**: Regex-based protection masks that mark these spans as "untouchable":
- JSON objects/arrays (nested, multiline, escaped)
- Code blocks (`` ```...``` ``)
- File paths (starts with `/` or `./`)
- Identifiers (camelCase, snake_case, UPPER_CASE)
- Domain terms (configurable: "Vectorizer", "Synap", "UMICP")

**Cost**: Negligible (<0.1% latency overhead); protection spans typically <10% of text.

**Benefit**: Enables safe compression of API responses, config examples, technical documentation.

---

## Tokenizer as Pluggable Trait, Not Hardcoded

### Decision

Abstract tokenization via trait, allowing multiple implementations (MockTokenizer, future: Claude, GPT-4, etc.).

### Rationale

**Requirement**: Different LLMs have different tokenization strategies (BPE variants, SentencePiece, etc.).

**Single-Tokenizer weakness**: Would lock solution to one LLM's tokenizer, breaking portability.

**Trait-based strength**:
- Users can swap tokenizers without changing compression logic
- Scoring remains stable across LLMs (IDF, position, etc. are tokenizer-agnostic)
- Easy to add new tokenizers (Claude, GPT-4, Mistral) as external deps

**Current Gap**: Mock tokenizer is used in production. Real tokenizers (Claude, GPT-4) are pending external crate availability.

---

## Contextual Stopword Preservation

### Decision

Instead of removing ALL stopwords (e.g., "in", "to", "how"), preserve them in certain contexts (e.g., "how to", "in src/").

### Rationale

**Problem**: Hard stopword removal breaks meaning:
- "how to" → "how" [removed "to"]
- "in src/" → [removed "in"] "src/"

**Solution**: Keep stopword if:
1. Preceded by interrogative/imperative ("how", "what", "show")
2. Part of technical phrase ("in src/", "to compile")
3. Negation ("not", "no", "never")
4. Comparative ("as", "than", "like")

**Cost**: Minimal (simple context window check).

**Benefit**: Preserves sentence structure and idioms while still removing 90% of traditional stopwords.

---

## No External ML Models or Fine-Tuning

### Decision

Pure rule-based scoring; no external models, no fine-tuning, no API calls.

### Rationale

**Requirements from Cortex**:
- Offline operation (no network dependency)
- Deterministic behavior (same input → same output)
- Sub-millisecond latency
- Suitable for real-time workers

**ML Model approach weakness**:
- Requires external service or fine-tuned weights
- Non-deterministic (if sampling-based)
- High latency (API call or GPU inference)
- Adds operational complexity

**Rule-based strength**:
- Fully offline, single-machine operation
- Deterministic (reproducible)
- <1ms latency
- Zero operational overhead

**Quality trade-off**: 89% quality (statistical) vs. potential 95%+ (fine-tuned model). Acceptable for Cortex's use case (cost optimization, not perfection).

---

## Validation: Real arXiv Papers Over Synthetic Data

### Decision

Benchmark against 200 real academic papers from arXiv, not generated synthetic text.

### Rationale

**Synthetic weakness**:
- May not reflect real repetition patterns
- Easy to achieve high metrics (cherry-picked examples)
- Doesn't represent actual use cases (RAG context, documentation)

**Real data strength**:
- Diverse vocabulary, structure, length
- Genuine repetition patterns (citations, sections)
- Reflects actual Cortex workloads
- Enables ground-truth validation (questions about papers)

**Scale**: 1.6M tokens (200 papers) large enough to validate scaling properties.

---

## A/B Testing on 6 LLMs Rather Than 1

### Decision

Validate quality across Grok-4, Claude Sonnet, Claude Haiku, GPT-5, Gemini Pro, Grok (6 models, 350+ test pairs).

### Rationale

**Single-LLM weakness**:
- Quality varies by model (Claude might rate 91%, but GPT-5 might rate 85%)
- Users may prefer different models
- Doesn't reflect production diversity

**Multi-LLM strength**:
- Demonstrates robustness across tokenization strategies
- Provides model-specific recommendations (e.g., "Claude Haiku prefers statistical_70")
- Gives users confidence across their tech stack

**Result**: High consistency (89–93% quality across all models), validating algorithm robustness.

---

## Rust Over Python for Core Library

### Decision

Core compression logic in Rust; Python/TypeScript wrappers for higher-level APIs.

### Rationale

**Rust strength**:
- Compiled (10+ MB/s throughput vs. Python's 1–2 MB/s)
- Zero-cost abstractions (no GC pauses)
- Suitable for production workers
- Memory safety (no buffer overflows)
- Thread-safe by default (Cortex workers are concurrent)

**Python weakness** (for core):
- Interpreted (slower)
- GC pauses (problematic for latency-critical paths)
- Not ideal for worker integration

**Python role**: Examples, scripts, dataset generation, validation (where speed is less critical).

**Result**: v0.1.2 is pure Rust; Python/TypeScript ports are third-priority.
