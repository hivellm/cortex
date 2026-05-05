# TransmutationLite Consolidation Knowledge Base

Consolidated documentation for HiveLLM's TransmutationLite project, prepared for ingestion into Cortex.

**Status**: Production Ready (v0.6.2)  
**Last updated**: 2025-10-27  
**Formats supported**: 6 (PDF, DOCX, XLSX, PPTX, HTML, TXT)  
**Test coverage**: 177/177 (100%)

## Files in This Consolidation

| # | File | Purpose | Lines |
|---|------|---------|-------|
| 01 | [overview.md](01-overview.md) | Project purpose, stack, maturity | Purpose + role + maturity status |
| 02 | [architecture.md](02-architecture.md) | System design, components, data flow | Modules + DDD + dependencies |
| 03 | [public-surface.md](03-public-surface.md) | APIs, SDKs, CLIs | Library exports + CLI commands + npm package |
| 04 | [data-and-storage.md](04-data-and-storage.md) | Data models, schemas, storage | ConversionResult + metadata + cache + validation |
| 05 | [integrations.md](05-integrations.md) | Relationships to other projects | Transmutation (Rust) + Classify + Vectorizer + Nexus |
| 06 | [decisions-and-rationale.md](06-decisions-and-rationale.md) | Why "Lite"; what kept/dropped | Design trade-offs + pattern rationale |
| 07 | [operational.md](07-operational.md) | Docker, ports, env, build, deploy | Installation + build + testing + CLI + monitoring |
| 08 | [cortex-relevance.md](08-cortex-relevance.md) | Ingestion priorities for Cortex | What Cortex needs + integration workflow + metrics |
| 09 | [open-questions.md](09-open-questions.md) | Gaps, unknowns, risks | Documentation gaps + feature gaps + security gaps |

## Quick Facts

- **Language**: TypeScript 5.7.2
- **Runtime**: Node.js ≥18.0.0
- **Distribution**: npm (@hivehub/transmutation-lite)
- **Publication**: Ready but not yet published
- **Maturity**: Production Ready (v0.6.2, all 177 tests passing)
- **License**: MIT
- **Repository**: https://github.com/hivellm/transmutation-lite

## Key Insights for Cortex

### What TransmutationLite Does

Converts documents (PDF, DOCX, XLSX, PPTX, HTML, TXT) to Markdown with metadata extraction.

### Why Cortex Cares

- **Input source**: Documents → Cortex uses TransmutationLite to normalize to Markdown
- **Metadata source**: Extracted format, pageCount, author, createdAt feeds Cortex faceting
- **Performance**: Batch throughput directly affects Cortex indexing speed
- **Reliability**: Conversion failures affect Cortex index completeness

### Integration Points

1. **Classify**: Uses TransmutationLite to convert docs before LLM classification
2. **Vectorizer**: Receives Markdown from TransmutationLite; produces embeddings
3. **Cortex**: Indexes Markdown (text) + embeddings (vectors) + metadata (graph)
4. **Nexus**: Stores metadata in graph; enables faceted search

### Critical Decisions

1. **TypeScript over Rust**: Fast integration > peak performance for classification
2. **No OCR in Lite**: Accepted precision loss; full Transmutation available for high-quality RAG
3. **Markdown only**: Single output format; sufficient for downstream systems
4. **Caching optional**: Recommended for repeated conversions; not forced

## How to Use This Knowledge Base

### For Cortex Developers

Start with:
1. **[08-cortex-relevance.md](08-cortex-relevance.md)** — What Cortex needs from TransmutationLite
2. **[03-public-surface.md](03-public-surface.md)** — How to call TransmutationLite
3. **[04-data-and-storage.md](04-data-and-storage.md)** — Data structures

### For Documentation

Start with:
1. **[01-overview.md](01-overview.md)** — Purpose and maturity
2. **[02-architecture.md](02-architecture.md)** — System design
3. **[05-integrations.md](05-integrations.md)** — Ecosystem relationships

### For Operations

Start with:
1. **[07-operational.md](07-operational.md)** — Installation, build, testing, deployment
2. **[04-data-and-storage.md](04-data-and-storage.md)** — Limits and performance
3. **[09-open-questions.md](09-open-questions.md)** — Known gaps and monitoring

### For Architecture Review

Start with:
1. **[06-decisions-and-rationale.md](06-decisions-and-rationale.md)** — Why design choices
2. **[02-architecture.md](02-architecture.md)** — Detailed system design
3. **[09-open-questions.md](09-open-questions.md)** — Security and performance gaps

## Data Flow (Quick Reference)

```
Source Document (PDF, DOCX, XLSX, PPTX, HTML, TXT)
  ↓
TransmutationLite.convertFile(path) or CLI
  ↓
ConversionResult {
  markdown: string,
  metadata: { format, pageCount, author, ... },
  conversionTimeMs: number,
  warnings?: string[]
}
  ↓
Vectorizer.embed(markdown)
  ↓
Cortex.index({
  markdown,
  metadata,
  embeddings
})
  ↓
Meilisearch (text search) + Nexus (graph + vector search)
```

## Format Support Summary

| Format | Supported | Quality | Loss | Recommendation |
|--------|-----------|---------|------|-----------------|
| PDF | ✅ | Basic | Images | OK for text-heavy docs |
| DOCX | ✅ | Good | Images | Good for reports |
| XLSX | ✅ | Good | Formulas | Good for tables |
| PPTX | ✅ | Limited | Layout, notes | Text extraction only |
| HTML | ✅ | Good | JS/styles | Semantic Markdown |
| TXT | ✅ | Good | None | Direct passthrough |

## Performance Summary

| Operation | Typical Time | Memory |
|-----------|--------------|--------|
| Convert 2 MB PDF | 200–500 ms | ~50 MB |
| Convert 500 KB DOCX | 150–300 ms | ~30 MB |
| Batch 4 files parallel | ~500 ms | ~150 MB |
| Cache hit | <1 ms | Negligible |

## Known Limitations

1. **No OCR**: Image-only PDFs cannot be converted
2. **No streaming**: Files loaded entirely into memory (500 MB limit)
3. **PPTX limited**: Basic text extraction only (no layout)
4. **Images dropped**: From PDF, DOCX conversions (not extracted)
5. **Security pending**: 7 vulnerabilities in dependencies (not yet audited)

## Critical Actions for Cortex Integration

### Before Production

- [ ] Security audit of dependencies (7 vulnerabilities)
- [ ] Performance testing with real Classify workload
- [ ] Plan Cortex tasks for document conversion pipeline

### After Production

- [ ] Monitor conversion failure rates
- [ ] Track performance (avg time, cache hit rate)
- [ ] Establish fallback to full Transmutation for OCR-required docs

## Related Documentation

- **TransmutationLite repository**: https://github.com/hivellm/transmutation-lite
- **Full Transmutation (Rust)**: https://github.com/hivellm/transmutation
- **HiveLLM Classify**: Uses TransmutationLite for document conversion
- **Cortex**: Indexes converted documents from TransmutationLite

---

**Prepared for**: HiveLLM Cortex consolidation  
**Date**: 2025-10-27  
**Format**: 9 Markdown files, ≤250 lines each, factual and sourced
