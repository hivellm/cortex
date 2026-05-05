# Transmutation — Data & Storage

## Input Formats (27 supported)

### Documents (12 core + beta)
| Format | Engine | Mode | Status |
|--------|--------|------|--------|
| PDF | pdf-extract (pure Rust) | Fast, Precision, FFI | ✅ Production |
| DOCX | docx-rs (pure Rust) | Text, Images | ✅ Production |
| XLSX | umya-spreadsheet (pure Rust) | Tables, CSV | ✅ Production, 148 pg/s |
| PPTX | umya-spreadsheet (pure Rust) | Per-slide text, images | ✅ Production, 1639 pg/s |
| HTML | scraper + html5ever (pure Rust) | DOM parsing | ✅ Production, 2110 pg/s |
| XML | quick-xml (pure Rust) | Tree parsing | ✅ Production, 2353 pg/s |
| TXT | Native | Plain text | ✅ Production, 2805 pg/s |
| CSV/TSV | Native + comrak | Delimiter parsing | ✅ Production, 2647 pg/s |
| RTF | Simplified parser (pure Rust) | Text extraction | ⚠️ Beta |
| ODT | ZIP + XML (pure Rust) | ODF container parsing | ⚠️ Beta |
| MD | Planned | Markdown normalization | 🔄 Planned |
| ZIP | zip crate (pure Rust) | Archive listing, file extraction | ✅ Production, 1864 pg/s |

### Images (6 formats + OCR)
Formats: JPEG, PNG, TIFF, BMP, GIF, WEBP  
OCR Engine: Tesseract (optional)  
Output: Markdown (text), JSON (text + confidence)

### Audio (5 formats)
Formats: MP3, WAV, M4A, FLAC, OGG  
Transcription Engine: Whisper CLI (optional)  
Output: Markdown (transcript), JSON (text + timing metadata)

### Video (5 formats)
Formats: MP4, AVI, MKV, MOV, WEBM  
Extraction: FFmpeg → audio extraction → Whisper  
Output: Markdown (transcript), JSON (text + timing)

### Archives (Extended support optional)
- **ZIP:** Always enabled (native)
- **TAR/GZ:** Optional (tar + flate2 crates)
- **7Z:** Optional (sevenz-rust crate)

## Output Formats

### 1. Markdown
**Use:** LLM consumption, RAG ingestion, human reading  
**Features:**
- Heading hierarchy (`#`, `##`, `###`, etc.)
- Lists, tables, code blocks, blockquotes
- Optional per-page splitting (one file per page)
- Metadata as YAML frontmatter (optional)
- LLM optimization: chunks with overlap, normalized whitespace

**Example:**
```markdown
# Document Title

## Section 1

Extracted text with preserved paragraph structure.

| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
```

### 2. JSON
**Use:** Structured data consumption, API responses  
**Schema:**
- `content: string` — main text (Markdown)
- `metadata: Object` — file size, page count, creation date, etc.
- `pages: Array[Object]` (if split mode) — per-page text + coordinates
- `images: Array[Object]` (if extracted) — base64-encoded + alt text
- `tables: Array[Object]` (if extracted) — CSV + JSON serialization

**Example:**
```json
{
  "content": "# Title\n\nParagraph text.",
  "metadata": {
    "source": "document.pdf",
    "pages": 15,
    "created_at": "2026-01-15T10:00:00Z"
  },
  "pages": [
    { "number": 1, "content": "...", "height": 792, "width": 612 }
  ]
}
```

### 3. CSV
**Use:** Tabular data, spreadsheet export  
**Features:**
- Configurable delimiter (`,`, `;`, `\t`)
- Optional headers
- Automatic escaping for quoted fields

### 4. Images (PNG, JPEG, WEBP)
**Use:** Visual preservation, OCR source  
**Options:**
- DPI: 150 (default), 300 (high quality)
- Quality: Low (80), Medium (85), High (95)
- Format: PNG (lossless), JPEG (lossy), WEBP (modern)

## Storage Patterns

### Caching (Optional)
- **Key:** SHA256(file path + options)
- **Value:** Serialized `ConversionResult`
- **Location:** `$XDG_CACHE_HOME/transmutation/` (Linux/macOS) or `%APPDATA%\Transmutation\cache\` (Windows)
- **TTL:** Unbounded (manual cleanup)

### Temp Files
- **Location:** System temp directory (via `tempfile` crate)
- **Scope:** Per-conversion, auto-cleanup on drop
- **Use:** Intermediate extraction (e.g., archives, video processing)

## Memory Characteristics

| Phase | Memory | Notes |
|-------|--------|-------|
| **Initialization** | ~5MB | Static regex patterns, model cache (FFI only) |
| **Per-conversion baseline** | ~20MB | Input buffer, output builder |
| **Large PDF (100 pages)** | ~50–100MB | Full text in memory, optimization buffers |
| **Image extraction** | +50–100MB per image | Image rasterization (temporary) |
| **Peak (split-pages mode)** | O(max_page_size) | Pages extracted one at a time (v0.3.0+ fix) |

**Optimization in v0.3.0:**
- Cached regex patterns (11 total, compiled once)
- Pre-allocated buffers with 20% overhead estimate
- O(n) page extraction instead of O(n²)
- Early memory release of PDF bytes after text extraction

## Serialization

All `ConversionResult` structs use `serde` for JSON serialization. Custom `Serialize` implementations for:
- `FileFormat` — string representation
- `OutputFormat` — nested object
- `ConversionOptions` — flat or nested per context

**No binary formats used.** All output is text-based (Markdown, JSON, CSV) or image (PNG/JPEG/WEBP).
