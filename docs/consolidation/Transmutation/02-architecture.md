# Transmutation — Architecture

## System Layers

```
CLI / API (transmutation convert, transmutation batch)
    ↓
High-Level Converter API (Converter, ConversionBuilder, BatchProcessor)
    ↓
Format-Specific Converters (PDF, DOCX, XLSX, PPTX, HTML, XML, Image, Audio, Video, Archive)
    ↓
Engine Layer (PDF extraction, Tesseract, Whisper, FFmpeg, docling-parse FFI)
    ↓
Output Formatters (Markdown, JSON, CSV, Images)
    ↓
Optimization (Text cleanup, deduplication, LLM chunking)
    ↓
Result (ConversionResult with metadata, timing, quality metrics)
```

## Core Components

### 1. Converter Trait (async)
Primary abstraction for all document converters. Implementations:
- `PdfConverter` — pure Rust (pdf-extract) + optional FFI
- `DocxConverter` — pure Rust (docx-rs)
- `XlsxConverter` — pure Rust (umya-spreadsheet)
- `PptxConverter` — pure Rust (umya-spreadsheet)
- `HtmlConverter` — pure Rust (scraper, html5ever)
- `XmlConverter` — pure Rust (quick-xml, roxmltree)
- `ImageConverter` — Tesseract OCR (optional)
- `AudioConverter` — Whisper CLI (optional)
- `VideoConverter` — FFmpeg + Whisper (optional)
- `ArchiveConverter` — ZIP native, TAR/GZ optional

### 2. Output Formats
```rust
pub enum OutputFormat {
    Markdown { split_pages: bool, optimize_for_llm: bool },
    Image { format: ImageFormat, quality: ImageQuality, dpi: u32 },
    Json { structured: bool, include_metadata: bool },
    Csv { delimiter: char, include_headers: bool },
    EmbeddingReady { max_chunk_size: usize, overlap: usize },
}
```

### 3. Data Pipeline
- **Detection:** File type auto-detection via magic bytes + extension
- **Conversion:** Format-specific converter produces intermediate representation
- **Optimization:** Text cleanup, deduplication, chunking for LLM consumption
- **Export:** Serialization to target format
- **Caching:** Hash-based cache for repeated conversions (optional)

### 4. Batch Processing
`BatchProcessor` manages parallel conversions using Tokio. Configurable parallelism (default: CPU count). Progress tracking and error collection per file.

## Conversion Modes

| Mode | Implementation | Quality | Speed | Use Case |
|------|----------------|---------|-------|----------|
| **Fast** | Pure Rust heuristics | 80% | 250x faster | High-volume ingestion |
| **Precision** | Enhanced heuristics + space correction | 77% | 250x faster | Production (default) |
| **FFI** (future) | C++ docling-parse + ONNX ML | 95%+ | ~50x faster | Research, legal |

## Key Design Decisions

1. **Pure Rust Core:** Core 8 formats (PDF, DOCX, XLSX, PPTX, HTML, XML, TXT, ZIP) require **zero external dependencies**. Optional features (OCR, ASR, video) use external tools (Tesseract CLI, Whisper CLI, FFmpeg) rather than Rust crates to avoid bloat.

2. **Modular Features:** File format support is feature-gated (`office`, `tesseract`, `audio`, `video`, `archives-extended`). CLI requires explicit `cli` flag to avoid build overhead for library users.

3. **Memory-First Optimization:** v0.3.0+ uses cached regex patterns, pre-allocated buffers, and O(n) page extraction to minimize heap pressure, especially important for library usage (e.g., in Cortex workers).

4. **No External SDKs:** Transmutation is self-contained. It does not depend on Vectorizer SDK, Cortex SDK, or any HiveLLM services. It is a pure conversion tool; downstream integration is Cortex's responsibility.

## Extensibility

New converters:
1. Implement `Converter` trait async methods
2. Register in `converters/mod.rs`
3. Add to `FileFormat` enum
4. Update feature gates if optional

New output formats:
1. Extend `OutputFormat` enum
2. Implement in `output/` module
3. Update `ConversionResult` serialization
