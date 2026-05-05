# Transmutation — Overview

**Version:** 0.3.2  
**Status:** Production  
**Repository:** https://github.com/hivellm/transmutation

## Purpose

Transmutation is a high-performance document conversion engine that transforms 27+ file formats into LLM-optimized text, image, and JSON outputs. Designed as a Rust alternative to Docling, it achieves **98x faster** conversion while maintaining acceptable quality (80–95% similarity depending on mode).

## Role in HiveLLM

Transmutation sits upstream of the vectorization pipeline. It handles format normalization and extraction, feeding clean text and images to Vectorizer for embedding generation. It is **NOT** a downstream service; it is a library/CLI tool that runs as part of document ingestion.

## Core Stack

- **Language:** Rust 1.85+ (Edition 2024)
- **Async Runtime:** Tokio 1.47
- **Document Parsing:** Pure Rust (pdf-extract, docx-rs, umya-spreadsheet, scraper)
- **Optional Engines:** Tesseract (OCR), Whisper (ASR), FFmpeg (video), docling-parse FFI
- **Output:** Markdown, JSON, CSV, Images
- **Parallelism:** Tokio for async, Rayon for CPU-bound tasks

## Maturity & Status

| Aspect | Status | Notes |
|--------|--------|-------|
| **Core (PDF, DOCX, XLSX, PPTX, HTML, XML, ZIP)** | ✅ Production | 100% complete, high quality |
| **Extended (RTF, ODT)** | ⚠️ Beta | Working, room for improvement |
| **OCR (Tesseract, 6 formats)** | ✅ Production | 88x faster than Docling |
| **Audio/Video (Whisper, FFmpeg)** | ✅ Production | Transcription support |
| **Precision Mode (FFI)** | 🚧 In Development | C++ docling-parse bindings for 95%+ similarity |

**Version Timeline:**
- v0.1.0 (Oct 2025): MVP, PDF only, 98x benchmark
- v0.2.0 (Nov 2025): CI hardening
- v0.3.0 (Dec 2025): Memory optimizations, O(n) page extraction
- v0.3.2 (Feb 2026): Windows lib dependency fixes, FFI clarity on macOS/Linux

## Key Metrics

| Metric | Value |
|--------|-------|
| **Startup Time** | <100ms |
| **PDF Processing** | 71 pages/sec (Fast mode) |
| **Memory Footprint** | ~20MB (core), 50–100MB per conversion |
| **Binary Size** | 5MB (CLI, pure Rust) |
| **Quality vs Docling** | 80% (Fast) / 77% (Precision) / 95%+ (FFI) |
