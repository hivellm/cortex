# Transmutation — Cortex Relevance

## Why Cortex Needs Transmutation

Cortex's mission is to ingest, index, and search unstructured documents. Transmutation solves the **first mile** (format normalization):

```
Raw Documents (27 formats)
    ↓ [Transmutation]
LLM-Optimized Text (Markdown)
    ↓ [Cortex Consolidator]
Chunks (512-token windows)
    ↓ [Vectorizer]
Embeddings
    ↓ [Nexus]
Vector Search
    ↓ [Cortex Graph/API]
End-user queries
```

**Without Transmutation:** Cortex would need to implement 27 format converters in Rust, maintain them, and debug format-specific edge cases. **With Transmutation:** One battle-tested crate, pure Rust, zero Python.

## Integration Touchpoints

### 1. cortex-consolidator Worker
**Current Usage:** Likely invokes Transmutation CLI  
**Recommended Evolution:** Link Transmutation as library for in-process conversions

**Code pattern:**
```rust
// Current (CLI)
let output = tokio::process::Command::new("transmutation")
    .args(&["convert", &input_path, "-o", &output_path])
    .output()
    .await?;

// Recommended (library)
use transmutation::{Converter, OutputFormat};

let converter = Converter::new()?;
let result = converter
    .convert(&input_path)
    .to(OutputFormat::Markdown { split_pages: true, optimize_for_llm: true })
    .execute()
    .await?;

let markdown = result.content();
```

**Benefits:**
- No subprocess overhead
- Better error context
- Direct memory-to-memory (faster for batches)
- Single async runtime (Tokio shared)

### 2. cortex-embedder Worker (Hypothetical)
**Purpose:** After Transmutation converts, prepare text for Vectorizer  
**Pattern:** Read Markdown, chunk, send to Vectorizer API

**Integration Point:**
```rust
// Receive converted Markdown from cortex-consolidator
let markdown = /* from consolidator */;

// Chunk it for embedding
use transmutation::output::Chunker;  // If exported
let chunks = chunk_markdown(&markdown, max_size=512)?;

// Send to Vectorizer
for chunk in chunks {
    vectorizer.embed(&chunk).await?;
}
```

### 3. Storage Integration (cortex-storage)
**Pattern:** Store intermediate Markdown (optional, for observability)

**Use Case:** If a document fails embedding, re-run from stored Markdown without re-converting.

**Storage Schema:**
```rust
struct DocumentConversion {
    document_id: String,
    source_path: String,
    source_format: FileFormat,
    converted_markdown: String,
    metadata_json: serde_json::Value,
    conversion_time_ms: u64,
    created_at: DateTime<Utc>,
}
```

**Query Pattern:**
```sql
SELECT converted_markdown FROM document_conversions 
WHERE document_id = ? AND created_at > (NOW() - INTERVAL 7 DAY)
```

### 4. Monitoring & Observability
**Metrics to emit from Cortex workers:**
- `transmutation.convert.duration_ms` (histogram)
- `transmutation.convert.pages_processed` (counter)
- `transmutation.convert.errors_total` (counter by format)
- `transmutation.batch.throughput_files_per_sec` (gauge)

**Logs:**
```json
{
  "timestamp": "2026-02-28T10:15:42Z",
  "worker": "cortex-consolidator",
  "operation": "transmutation.convert",
  "document_id": "doc-abc123",
  "input_format": "pdf",
  "input_size_bytes": 2200000,
  "duration_ms": 210,
  "pages": 15,
  "status": "success",
  "output_size_bytes": 85000
}
```

## Ingestion Priorities

### Phase 1: Core Formats (Immediate)
**Formats to support:** PDF, DOCX, XLSX, PPTX, HTML, XML, TXT, ZIP  
**Status:** All stable in Transmutation v0.3.2  
**Action:** Cortex consolidator should handle these immediately  
**Quality:** 80% (Fast mode, default) to 77% (Precision mode)

### Phase 2: Images + OCR (Next)
**Formats to support:** JPEG, PNG, TIFF, BMP, GIF, WEBP (with Tesseract)  
**Status:** Stable in v0.3.2, requires `tesseract` feature  
**Action:** Enable `image-ocr` feature in Cortex's dependency tree  
**Quality:** 70–90% depending on image quality, language, layout  
**Note:** Requires external Tesseract binary installation

### Phase 3: Audio/Video (Optional)
**Formats to support:** MP3, WAV, M4A (audio), MP4, MKV (video)  
**Status:** Stable in v0.3.2, requires external Whisper and FFmpeg  
**Action:** Enable `audio`, `video` features only if document sources include recordings  
**Quality:** Depends on audio codec and language  
**Note:** Slower (Whisper inference time), may require GPU

### Phase 4: Extended Archives (Low Priority)
**Formats:** TAR, GZ, 7Z (ZIP is always enabled)  
**Status:** Stable in v0.3.2, requires `archives-extended` feature  
**Action:** Enable if document sources are compressed archives  
**Quality:** N/A (archive → extraction → format-specific conversion)

### Phase 5: Precision Mode / FFI (Future)
**Status:** Planned for v0.4.0, not ready for production  
**Action:** Monitor Transmutation roadmap; adopt when available  
**Benefit:** 95%+ similarity for legal/academic use cases  
**Trade-off:** Slower (50x faster than Docling, not 250x), requires C++ library

## Data Flow for Cortex

```
Document Source
    │
    ├─→ [cortex-api] Ingest endpoint (webhook, S3, etc.)
    │
    ├─→ [cortex-storage/Archive] Store raw document
    │
    ├─→ [cortex-consolidator] Task: convert_and_chunk
    │   ├─→ [Transmutation] Convert → Markdown
    │   └─→ [cortex-storage/Archive] Store converted Markdown (optional)
    │
    ├─→ [cortex-graph] Create Document node + metadata
    │
    ├─→ [cortex-embedder] Chunk Markdown + send to Vectorizer
    │
    ├─→ [Vectorizer] Generate embeddings
    │
    ├─→ [Nexus] Store vectors
    │
    └─→ [cortex-graph] Link embeddings to Document node
```

## Feature Adoption Strategy

### Minimum Viable (MVP)
- `transmutation = "0.3.2"` with default features (Core formats only)
- cortex-consolidator: CLI invocation for PDF → Markdown
- cortex-storage: Store converted Markdown (optional)

### Recommended (v1)
- `transmutation = { version = "0.3", features = ["office"] }` (DOCX, XLSX, PPTX support)
- Library API (in-process, not CLI)
- Batch processing with 4–8 parallel workers
- Metrics emission (duration, throughput)

### Advanced (v2)
- Add `image-ocr` feature for document scanning
- Implement cache layer (avoid re-converting identical inputs)
- Monitor OCR confidence scores for quality assurance
- Auto-retry on failures (transient vs permanent)

### Future (v3)
- Monitor Transmutation v0.4.0 release (Precision mode / FFI)
- Evaluate ROI: +15% quality vs 3–5x slower performance
- Enable for high-value document types (contracts, research papers)
- Disable for bulk ingestion (news, blog archives)

## Risk & Limitations

### Transmutation Risks
1. **Format Edge Cases:** 27 formats, each with variants. Some PDFs/DOCX files may fail or produce garbled output.
   - **Mitigation:** Test with representative documents before ingestion; monitor error rates.

2. **Precision Trade-off:** 77–80% similarity means headers, footers, marginal notes may be extracted incorrectly.
   - **Mitigation:** Use Precision mode for high-value docs; accept Fast mode for bulk.

3. **External Dependencies:** OCR (Tesseract), ASR (Whisper), video (FFmpeg) require external binaries.
   - **Mitigation:** Document installation; provide Docker image with all optional tools.

4. **Memory Spikes:** Large PDFs (>500 pages) or image extraction can spike to 200–300MB.
   - **Mitigation:** Use `--split-pages` for very large documents; monitor worker memory limits.

### Cortex Responsibilities
- **Format Validation:** Reject unsupported formats before invoking Transmutation.
- **Error Handling:** Graceful fallback for conversion failures (skip document or retry).
- **Storage Management:** Clean up intermediate Markdown files after embedding (or archive for debugging).
- **Version Pinning:** Keep Transmutation updated within 0.3.x range; test before upgrading major versions.

## Success Criteria

Cortex can consider Transmutation integration successful when:
1. ✅ 80% of documents in sample corpus convert without error
2. ✅ Consolidated text is suitable for LLM embedding (human spot-check)
3. ✅ Bulk ingestion throughput: ≥50 docs/sec (with 4 parallel workers)
4. ✅ Memory footprint: <500MB peak per worker
5. ✅ No production outages due to Transmutation failures

**Validation:** Set up integration tests with real document samples; measure before/after embedding quality.
