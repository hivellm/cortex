# Transmutation — Integrations

## HiveLLM Ecosystem Position

```
Document Sources (files, APIs, archives)
    ↓
Transmutation (format conversion)
    ↓
Text/Image outputs (Markdown, JSON)
    ↓
Vectorizer (embedding generation) ← **via Cortex workers**
    ↓
Nexus (vector DB storage)
    ↓
Cortex (search + indexing)
```

**Key Point:** Transmutation is upstream of Vectorizer. It does NOT integrate directly with Vectorizer SDK, Nexus SDK, or Cortex. Those integrations are Cortex's responsibility (cortex-consolidator, cortex-embedder, cortex-graph workers).

## Relationship to TransmutationLite

**TransmutationLite** (if it exists as a separate project):
- Not found in current directory structure
- Presumably a lightweight variant or earlier prototype
- Transmutation v0.3.2 is the **current production version**
- No active split or alternate maintenance

**Assumption:** Any "Lite" variant would be obsolete. Use Transmutation v0.3.2 for all new work.

## HiveLLM Services Transmutation Uses

**None.** Transmutation is self-contained. It does not depend on:
- Vectorizer SDK
- Nexus SDK
- Synap SDK
- Lexum SDK
- Expert SDK
- Cortex SDK

This design enables:
- Standalone library usage without HiveLLM infrastructure
- Reproducible builds (no version matrix with other services)
- Clear separation of concerns (conversion ≠ embedding ≠ storage)

## HiveLLM Services That Use Transmutation

1. **Cortex (cortex-consolidator, cortex-embedder)**
   - Invokes `transmutation` CLI or library API
   - Passes converted text to Vectorizer for embedding
   - Cortex is responsible for orchestration and storage integration

2. **HiveLLM Vectorizer (future)**
   - May bundle Transmutation as a pre-processing step
   - No explicit dependency planned; integration is optional

## Language Bindings & SDKs

### Current
- **Rust:** Native crate, full feature support
- **CLI:** Pure Rust binary, no runtime dependencies (except optional tools)

### Planned (Future)
- **Python (PyO3):** Not implemented; requires FFI wrapper
- **JavaScript/TypeScript (Neon):** Not implemented; would bundle WASM or FFI
- **Go:** No plans documented
- **Java:** No plans documented

**Note:** PyO3 and Neon are mentioned in README.md examples (v0.2+) but not yet implemented. These would require separate crate versions and additional maintenance.

## External Tools (Optional Dependencies)

| Tool | Feature | Status | Fallback |
|------|---------|--------|----------|
| **Tesseract OCR** | `tesseract`, `image-ocr` | Optional | Skip OCR if missing |
| **Whisper CLI** | `audio`, `video` | Optional | Skip transcription if missing |
| **FFmpeg** | `video` | Optional | Skip video processing if missing |
| **poppler-utils** | `pdf-to-image` | Optional | Skip PDF→image if missing |
| **LibreOffice** | DOCX/XLSX image extraction | Optional | Extract text only |

**Detection:** `build.rs` auto-detects missing tools at compile time, provides installation guidance.

**No Hard Failures:** All external tool features are optional. Core formats work without them.

## Integration Patterns for Cortex

### Option 1: CLI Invocation (cortex-consolidator)
```bash
# In Cortex worker
transmutation convert /tmp/input.pdf -o /tmp/output.md --optimize-llm --precision
# Then read output.md and pass to Vectorizer
```

**Pros:** Simple, isolated process, no Rust dependency in worker  
**Cons:** Subprocess overhead, file I/O coupling

### Option 2: Library API (cortex-embedder, new crate)
```rust
// In new cortex-embedder crate
use transmutation::{Converter, OutputFormat};

let converter = Converter::new()?;
let result = converter
    .convert(&input_path)
    .to(OutputFormat::Markdown {
        split_pages: true,
        optimize_for_llm: true,
    })
    .execute()
    .await?;

// Pass result.content to Vectorizer
```

**Pros:** No subprocess, direct memory-to-memory, better control  
**Cons:** Adds Transmutation to Cortex's Cargo.toml

## Storage Integration

### Cortex + Transmutation Output

**Flow:**
1. Cortex receives document (file, API, webhook)
2. Cortex calls Transmutation (CLI or library)
3. Transmutation outputs Markdown + metadata JSON
4. Cortex stores intermediate output (optional, for debugging)
5. Cortex extracts text, chunks, sends to Vectorizer
6. Vectorizer generates embeddings
7. Cortex stores embeddings in Nexus

**Intermediate Storage Recommendation:**
- Store converted Markdown in `cortex-storage` (Archive crate) with doc ID
- Keep JSON metadata with conversion timestamp + input file hash
- Enable faster re-processing if embedding pipeline fails

### Caching Strategy

- **Input:** Document file (path + mtime + size hash)
- **Output:** Converted Markdown + JSON metadata
- **Cache Key:** SHA256(file_path + file_size + mtime + conversion_options)
- **Storage:** `cortex-storage` volume or local Cortex cache
- **TTL:** Unbounded (document doesn't change often)
- **Validation:** Re-hash on retrieval; invalidate if input file changes

## Security Boundaries

**Transmutation is a converter, not a filter.** It does not enforce:
- Content moderation
- PII detection or redaction
- Malware scanning
- Access control

**Cortex's responsibility:** Validate inputs before passing to Transmutation, sanitize outputs if required by policy.

## Performance Expectations

When invoked from Cortex:

| Format | Size | Time | Throughput |
|--------|------|------|------------|
| PDF | 2.2MB (15 pages) | 0.21s | 71 pages/sec |
| DOCX | Varies | <0.1s (text only) | >100 files/sec |
| XLSX | 1MB | <0.05s | ~200 MB/sec |
| HTML | 500KB | <0.01s | >50 MB/sec |
| Batch (8 parallel) | 100 files | ~2–5 sec | ~20–50 files/sec |

**Implication:** Transmutation is rarely the bottleneck. Embedding generation (Vectorizer) is typically 10–100x slower.
