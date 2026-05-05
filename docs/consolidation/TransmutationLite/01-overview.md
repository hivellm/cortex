# TransmutationLite — Overview

## Purpose

TransmutationLite is a **lightweight TypeScript document converter** designed to convert common document formats (PDF, DOCX, XLSX, PPTX, HTML, TXT) to Markdown. It serves as a simplified alternative to the full Transmutation Rust library, optimized for use cases that prioritize ease of integration and "good enough" quality over maximum precision.

### Primary Use Case

- Integration with **HiveLLM Classify** for document classification workflows
- Quick document previews and prototyping
- Node.js-only environments without Rust toolchain

### Trade-offs

- **Lower precision** than full Transmutation (no OCR, no images, basic PPTX)
- **Moderate performance** (Node.js overhead; no 98x speed advantage of Rust)
- **Lightweight dependencies** (pdf-parse, mammoth, xlsx, jszip, turndown)

## Stack

- **Language**: TypeScript 5.7.2 (full type safety)
- **Runtime**: Node.js ≥18.0.0
- **Build**: tsup (bundler), vitest (tests), ESLint (lint)
- **Package manager**: npm
- **Distribution**: npm package (@hivehub/transmutation-lite)

## Maturity

- **Status**: ✅ **Production Ready** (v0.6.2, 2025-10-27)
- **Test Coverage**: 177/177 tests passing (100%)
- **Documentation**: Complete (API, architecture, 5 examples)
- **CI/CD**: GitHub Actions (test, lint, build, release workflows)
- **Publication**: Ready for npm; not yet published

## Quick Facts

| Metric | Value |
|--------|-------|
| Formats supported | 6 (PDF, DOCX, XLSX, PPTX, HTML, TXT) |
| Total LOC | ~1,200 (src/) |
| Test count | 177 |
| Package size | ~60 KB (compressed dist/) |
| Node requirement | ≥18.0.0 |
| License | MIT |
