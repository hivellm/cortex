# CompressionPrompt — Overview

## Purpose

CompressionPrompt is a **statistical prompt compression library** that reduces LLM token consumption by approximately 50% while maintaining 89-91% semantic quality. It operates model-free, using deterministic word importance scoring rather than external ML models or fine-tuning.

### Key Value Proposition

- **Cost Reduction**: 50% fewer tokens = 50% lower API costs ($2.50–$7.50/million saved)
- **Quality Preservation**: 89–93% quality (LLM-validated across 6 flagship models)
- **Production Speed**: <1ms per compression, 10+ MB/s throughput
- **No Dependencies**: Pure Rust, no external models or services required

## Role in HiveLLM Ecosystem

CompressionPrompt acts as a **context optimization layer** for any HiveLLM project sending large prompts to LLMs. Positioned upstream of LLM calls (e.g., within Cortex workers, Expert systems, or API routes), it reduces token count before transmission, cutting API costs without degrading output quality.

**Typical Integration Points**:
- Cortex consolidator/compressor workers (pre-LLM compression)
- RAG systems (compress retrieved chunks)
- Long-document processing pipelines
- Q&A system context reduction

## Technology Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| **Language** | Rust 1.85+ | Statically typed, zero-cost abstractions |
| **Core Algorithm** | Statistical Filtering | IDF + position + POS + entity + entropy scoring |
| **Tokenization** | Pluggable trait | MockTokenizer (test), future: Claude, GPT, Mistral |
| **Distribution** | Cargo crate | `compression-prompt v0.1.2` |
| **Validation** | 200 arXiv papers | 1.6M tokens, 350+ LLM test pairs |

## Current Version

**v0.1.2** (as of 2026-05-04)
- Statistical compression stable and production-ready
- JSON/code/structured-data protection added (v0.1.2 fix)
- Image rendering (beta, requires `image` feature)
- 41 test cases covering edge cases and structured data

## Metrics at a Glance

| Metric | Result |
|--------|--------|
| Token Savings | 50% (831K tokens from 1.6M original) |
| Quality (Claude Sonnet) | 91% |
| Quality (Grok-4) | 93% |
| Compression Speed | 0.92s for 1.6M tokens (10.58 MB/s) |
| Keyword Retention | 100% |
| Entity Retention | 91.8% |
| Test Coverage | 200 papers, 350+ A/B test pairs |
