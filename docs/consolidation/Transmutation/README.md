# Transmutation Consolidation Knowledge Base

This directory contains a comprehensive knowledge base about the **Transmutation** project for ingestion into Cortex's documentation system.

**Version:** Based on Transmutation v0.3.2 (Feb 2026)  
**Created:** 2026-05-04  
**Purpose:** Support Cortex integration, architectural decisions, and operational runbooks

## Files

| File | Audience | Purpose |
|------|----------|---------|
| **01-overview.md** | Everyone | What is Transmutation, why it exists, maturity status, key metrics |
| **02-architecture.md** | Engineers, maintainers | System design, components, data pipeline, extensibility |
| **03-public-surface.md** | Users, integrators | Library API, CLI, feature flags, Cargo examples, integration points |
| **04-data-and-storage.md** | Architects | Input/output formats, schemas, memory characteristics, serialization |
| **05-integrations.md** | Architects, integrators | HiveLLM ecosystem position, relationships to other services, integration patterns |
| **06-decisions-and-rationale.md** | Architects | Why key design choices were made, trade-offs, implications |
| **07-operational.md** | DevOps, operators | Docker, ports, env vars, logging, monitoring, troubleshooting, release process |
| **08-cortex-relevance.md** | Cortex team | Why Cortex needs Transmutation, ingestion priorities, data flow, risks, success criteria |
| **09-open-questions.md** | Researchers, planners | Known gaps, technical limitations, strategic questions |
| **README.md** | Everyone | This file; navigation guide |

## Quick Start

**Want to integrate Transmutation into Cortex?**
→ Start with [01-overview.md](01-overview.md), then [08-cortex-relevance.md](08-cortex-relevance.md), then [03-public-surface.md](03-public-surface.md).

**Want to understand the architecture?**
→ Read [02-architecture.md](02-architecture.md), then [04-data-and-storage.md](04-data-and-storage.md).

**Want to deploy Transmutation?**
→ Follow [07-operational.md](07-operational.md).

**Want to know design rationale?**
→ See [06-decisions-and-rationale.md](06-decisions-and-rationale.md).

**Want to identify risks and gaps?**
→ Check [09-open-questions.md](09-open-questions.md).

## Key Facts

- **Language:** Rust 1.85+ (Edition 2024)
- **Latest Version:** 0.3.2 (Feb 28, 2026)
- **Purpose:** High-performance document conversion (98x faster than Docling)
- **Input Formats:** 27 (8 core document formats, 6 image formats, 5 audio formats, 5 video formats, 3 archive formats)
- **Output Formats:** Markdown, JSON, CSV, Images (PNG, JPEG, WEBP)
- **Quality:** 80% (Fast mode) to 77% (Precision mode) to 95%+ (FFI mode, future)
- **Speed:** 250x faster than Docling for core formats
- **Memory:** ~50–100MB per typical conversion
- **Dependencies:** Zero for core formats; optional for OCR (Tesseract), ASR (Whisper), video (FFmpeg)

## For Cortex Maintenance

### When to Reference This Knowledge Base

1. **Integrating Transmutation into cortex-consolidator or cortex-embedder:**
   - Read [08-cortex-relevance.md](08-cortex-relevance.md) for integration patterns
   - Read [03-public-surface.md](03-public-surface.md) for library API

2. **Updating Transmutation dependency in Cortex:**
   - Check [06-decisions-and-rationale.md](06-decisions-and-rationale.md) for version pinning recommendation
   - Check [07-operational.md](07-operational.md) for deployment impact

3. **Troubleshooting Transmutation errors in Cortex:**
   - See [07-operational.md](07-operational.md) troubleshooting section
   - See [09-open-questions.md](09-open-questions.md) for known limitations

4. **Adding new document format support:**
   - Read [02-architecture.md](02-architecture.md) for extensibility patterns
   - Check [04-data-and-storage.md](04-data-and-storage.md) for format support status

5. **Performance tuning Cortex workers:**
   - See [07-operational.md](07-operational.md) performance tuning section
   - See [08-cortex-relevance.md](08-cortex-relevance.md) for recommended config

6. **Evaluating Transmutation limitations:**
   - See [09-open-questions.md](09-open-questions.md) for gaps and trade-offs
   - See [05-integrations.md](05-integrations.md) for no-dependency design explanation

## Sections by Role

### For Cortex API/Core Team
- [08-cortex-relevance.md](08-cortex-relevance.md) — Integration touchpoints, data flow, success criteria
- [07-operational.md](07-operational.md) — Docker, monitoring, troubleshooting

### For Cortex Workers Team
- [03-public-surface.md](03-public-surface.md) — How to call Transmutation from code
- [07-operational.md](07-operational.md) — Environment, timeouts, performance tuning
- [09-open-questions.md](09-open-questions.md) — Known edge cases, workarounds

### For DevOps/SRE
- [07-operational.md](07-operational.md) — Full runbook for deployment, monitoring, troubleshooting
- [05-integrations.md](05-integrations.md) — External tool dependencies (Tesseract, Whisper, FFmpeg)

### For Architects/Decision Makers
- [01-overview.md](01-overview.md) — Why Transmutation, not alternatives
- [02-architecture.md](02-architecture.md) — System design
- [06-decisions-and-rationale.md](06-decisions-and-rationale.md) — Trade-offs and implications
- [08-cortex-relevance.md](08-cortex-relevance.md) — ROI for Cortex

### For External Documentation
- All files can be published (factual, sourced, no sensitive information)
- Use this KB to answer user questions about Cortex + Transmutation integration

## Sources

All information in this KB is sourced from:
- Transmutation README.md, Cargo.toml, docs/*.md
- Transmutation source code (src/lib.rs, src/types.rs, src/converters/*)
- Transmutation CHANGELOG.md
- Official GitHub repository: https://github.com/hivellm/transmutation

No assumptions or speculative content. Links to external sources are provided where relevant.

## Maintenance

This KB should be updated when:
- Transmutation releases a new version (update version numbers, features, metrics)
- Cortex integration patterns change (update 08-cortex-relevance.md)
- New decisions are made about Transmutation's role in Cortex (update 06-decisions-and-rationale.md)
- Open questions are resolved (update 09-open-questions.md)

**Recommended Review Cycle:** Quarterly or per major Transmutation release (0.4.0+).

---

**Last Updated:** 2026-05-04  
**Author:** Cortex Documentation Agent  
**License:** CC0 (public domain) — use freely in Cortex documentation
