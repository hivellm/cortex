# Transmutation — Public Surface

## Library API

**Crate:** `transmutation` (crates.io)  
**Minimum Rust:** 1.85 (Edition 2024)

### Core Types

```rust
// Converter initialization
let converter = Converter::new()?;
let converter = Converter::with_config(ConverterConfig {
    enable_cache: true,
    max_parallel: num_cpus::get(),
    timeout: Duration::from_secs(300),
})?;

// Single file conversion (fluent API)
let result = converter
    .convert("document.pdf")
    .to(OutputFormat::Markdown {
        split_pages: true,
        optimize_for_llm: true,
    })
    .with_options(ConversionOptions { /* ... */ })
    .execute()
    .await?;

// Batch processing
let batch = BatchProcessor::new(converter);
let results = batch
    .add_files(&["doc1.pdf", "doc2.docx"])
    .to(OutputFormat::Markdown { split_pages: false, optimize_for_llm: true })
    .parallel(4)
    .execute()
    .await?;
```

### Key Enums

- `FileFormat` — 27 input formats (PDF, DOCX, XLSX, PPTX, HTML, XML, images, audio, video, archives)
- `OutputFormat` — Markdown, Image, Json, Csv, EmbeddingReady
- `ConversionResult` — output path, size, page count, timing, error details

## CLI

**Binary:** `transmutation` (requires `cli` feature)

### Commands

```bash
# Convert single file
transmutation convert <input> -o <output> [options]

# Batch conversion
transmutation batch <pattern> -o <output_dir> [options]

# Help
transmutation --help
transmutation convert --help
```

### Common Options

| Flag | Type | Example |
|------|------|---------|
| `-o, --output` | Path | `-o output.md` |
| `--format` | Format | `--format markdown` (default) |
| `--precision` | Flag | Use precision mode (77% quality) |
| `--split-pages` | Flag | Output one file per page |
| `--extract-images` | Flag | Extract images (if supported) |
| `--ocr` | Flag | Run OCR on images |
| `--parallel` | Count | `--parallel 4` (default: CPU count) |
| `--timeout` | Seconds | `--timeout 300` |

### Examples

```bash
# PDF to Markdown (fast mode, default)
transmutation convert paper.pdf -o paper.md

# PDF to Markdown (precision mode)
transmutation convert paper.pdf -o paper.md --precision

# Batch: convert all PDFs in folder
transmutation batch papers/*.pdf -o output/ --parallel 8

# Extract images per page
transmutation convert document.docx -o document.md --extract-images --split-pages

# OCR scanned document
transmutation convert scan.jpg -o scan.md --ocr --lang eng

# Archive extraction and conversion
transmutation convert archive.zip -o output/ --recursive
```

## Feature Flags

| Feature | Default | Enables |
|---------|---------|---------|
| `office` | ✅ Yes | DOCX, XLSX, PPTX text extraction |
| `pdf-to-image` | ❌ No | PDF → PNG/JPEG per page (pdfium-render) |
| `tesseract` | ❌ No | OCR for 6 image formats |
| `image-ocr` | ❌ No | Alias for tesseract |
| `audio` | ❌ No | Audio transcription (Whisper CLI) |
| `video` | ❌ No | Video transcription (FFmpeg + Whisper) |
| `archives-extended` | ❌ No | TAR, GZ, 7Z support |
| `docling-ffi` | ❌ No | C++ docling-parse + ONNX for 95%+ quality |
| `cli` | ❌ No | Build CLI binary |
| `full` | ❌ No | All optional features except CLI |

## Cargo Examples

```toml
# Library use (core formats only)
[dependencies]
transmutation = "0.3"

# Library with Office formats
[dependencies]
transmutation = { version = "0.3", features = ["office"] }

# Library with OCR
[dependencies]
transmutation = { version = "0.3", features = ["office", "image-ocr"] }

# CLI
[dependencies]
transmutation = { version = "0.3", features = ["cli"] }
```

## Integration Points

**No automatic integration with HiveLLM services.** Transmutation is invoked explicitly:
- By Cortex workers (cortex-workers/consolidator) for document ingestion
- By external systems via CLI or library API
- Output is passed to Vectorizer for embedding (Cortex's responsibility)

**Not a Vectorizer plugin**, not a Cortex service endpoint. It is a pure conversion tool.
