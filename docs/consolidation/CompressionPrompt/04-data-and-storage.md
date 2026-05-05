# CompressionPrompt — Data & Storage

## Benchmark Datasets

### arXiv Markdown Collection

**Source**: Real academic papers from arXiv converted to Markdown.

**Variants**:

| Scale | Papers | Size | Tokens | Use Case |
|-------|--------|------|--------|----------|
| Small | 100 | 5–10 MB | 1.5–3M | Development, quick tests |
| Medium | 200 | 10–20 MB | 3–6M | Standard benchmark |
| Large | 500 | 25–50 MB | 7–15M | Stress testing |
| XL | 1000 | 50–100 MB | 15–30M | Production simulation |

**Storage Location**: `benchmarks/datasets/arxiv_markdown/`

**Format**: Markdown with preserved structure (citations, equations, sections)

**Selection Criteria**:
- Mixed CS categories (cs.AI, cs.CL, cs.LG, cs.CV, cs.IR)
- Recent papers (2020–2024)
- Standard length (8–20 pages)
- Well-cited venues
- Exclude: Very short, books, excessive equations, non-English

### Concatenated Prompts

**Location**: `benchmarks/datasets/prompts/`

**Files**:
- `benchmark_100_papers.txt` — 100 paper concatenation
- `benchmark_200_papers.txt` — 200 paper concatenation
- `benchmark_500_papers.txt` — 500 paper concatenation
- `benchmark_1000_papers.txt` — 1000 paper concatenation

### LLM Evaluation Dataset

**Location**: `benchmarks/datasets/llm_evaluation/`

**Contents**: 63 compressed prompt pairs with metadata.

**Structure**:
```
llm_evaluation/
├── dataset.json                    # Master index
├── prompts/
│   ├── paper_001_statistical_30_metadata.json
│   ├── paper_001_statistical_50_metadata.json
│   ├── paper_001_statistical_70_metadata.json
│   ├── paper_001_dictionary_metadata.json
│   └── ... (63 pairs)
```

**Metadata Format**:
```json
{
  "arxiv_id": "2301.00001",
  "compression_method": "statistical_50",
  "compression_ratio": 0.5,
  "original_tokens": 2000,
  "compressed_tokens": 1000,
  "quality": {
    "keyword_retention": 0.92,
    "entity_retention": 0.89,
    "overall_score": 0.89
  },
  "performance": {
    "compression_time_ms": 0.16,
    "throughput_mbs": 10.58
  }
}
```

## A/B Test Results

**Location**: `benchmarks/ab_tests/`

**Coverage**: 350+ test pairs across 6 LLMs (Grok-4, Claude Sonnet, Grok, GPT-5, Gemini Pro, Claude Haiku).

**Files**:
- `ab_test_suite.json` — Master test results
- `ab_test_comparison.md` — Aggregated comparison
- `individual_tests/` — Per-paper per-technique JSON files

**Structure**:
```json
{
  "paper_id": 1,
  "technique": "statistical_50",
  "llm_results": {
    "grok-4": { "quality": 0.93, "hallucination": false },
    "claude-sonnet": { "quality": 0.91, "hallucination": false },
    "gpt-5": { "quality": 0.89, "hallucination": false },
    ...
  }
}
```

## Metadata & Paper Index

**Location**: `benchmarks/datasets/metadata/`

**Files**:
- `papers_100.json` — 100-paper metadata
- `papers_200.json` — 200-paper metadata
- `papers_500.json` — 500-paper metadata
- `papers_1000.json` — 1000-paper metadata

**Schema**:
```json
{
  "papers": [
    {
      "arxiv_id": "2301.00001",
      "title": "...",
      "authors": ["...", "..."],
      "year": 2023,
      "category": "cs.CL",
      "abstract": "...",
      "pdf_path": "arxiv_pdfs/2301.00001v1.pdf",
      "markdown_path": "arxiv_markdown/2301.00001v1.md"
    }
  ],
  "total_papers": 100,
  "total_tokens_estimate": 2000000,
  "categories": ["cs.CL", "cs.AI", "cs.LG"]
}
```

## Compression Results

**Location**: `benchmarks/results/compression/`

**Metrics Tracked**:
- Original token count
- Compressed token count
- Compression ratio
- Token savings
- Compression time
- Memory usage
- Dictionary characteristics (if applicable)
- Token retention (keywords, entities)

**Result Format**:
```json
{
  "dataset": "benchmark_200_papers",
  "method": "statistical_50",
  "original_tokens": 1662729,
  "compressed_tokens": 831364,
  "compression_ratio": 0.5,
  "token_savings": 831365,
  "compression_time_ms": 920,
  "throughput_mbs": 10.58,
  "quality_score": 0.886,
  "keyword_retention": 1.0,
  "entity_retention": 0.918
}
```

## Reproducibility

### Version Pinning

- Transmutation version (PDF → Markdown converter)
- Rust version: 1.85+
- Tokenizer version
- Random seed for paper selection: `42`

### Docker Reproducibility

Dockerfile provided for consistent environment:
```dockerfile
FROM rust:1.85-nightly
# Install transmutation, Python tools, run benchmark scripts
```

## Test Corpus Characteristics

### Expected High-Frequency N-grams (Bibliography Patterns)

- "In Proceedings of"
- "Conference on Neural Information Processing Systems"
- "arXiv preprint arXiv:"
- "et al."

### Section Headers (Repetitive)

- "## Introduction"
- "## Related Work"
- "## Experiments"
- "## Conclusion"

### Academic Phrases (Common)

- "we propose"
- "in this paper"
- "state-of-the-art"
- "experimental results show"

### Compressed Data Protection

Since v0.1.2, JSON objects, code blocks, file paths, and identifiers are **100% preserved** during compression:
- JSON structures (nested, multiline, escaped characters)
- Code blocks (`` ```code``` ``)
- File paths (`/path/to/file.ext`)
- Identifiers (`camelCase`, `snake_case`, `UPPER_CASE`)
- Domain terms (configurable)
