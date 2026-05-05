# TransmutationLite — Cortex Relevance

## Why Cortex Should Ingest TransmutationLite Knowledge

TransmutationLite is a **core component in the HiveLLM document processing pipeline**. Cortex must understand:

1. **What TransmutationLite converts** → Input to Vectorizer → Input to Cortex indexing
2. **Format support matrix** → Determines which documents Cortex can ingest
3. **Metadata extraction** → Fields Cortex can use for classification/filtering
4. **Performance characteristics** → Planning batch jobs in Cortex pipeline
5. **Integration points** → Classify → TransmutationLite → Vectorizer → Cortex

## Ingestion Priorities

### Priority 1: Document Conversion Specs

**What Cortex needs to know:**

- TransmutationLite converts 6 formats to Markdown
- Output is **always Markdown** (no other formats)
- Metadata structure: `{ format, fileSize, pageCount, title, author, createdAt, extra }`
- **No image extraction** in lite version (images lost in conversion)
- **Error handling**: `ConversionError` with format + cause

**Cortex action**: When indexing documents:
1. Detect format from file extension
2. Route to TransmutationLite (or full Transmutation if OCR needed)
3. Capture metadata for faceted search
4. Index Markdown content in Meilisearch
5. Store metadata in Nexus graph

### Priority 2: Format Support Matrix

**Critical for Cortex**:

| Format | Supported | Quality | Loss | Cortex Impact |
|--------|-----------|---------|------|--------------|
| PDF | ✅ | Basic | Images, some formatting | Full text indexing OK |
| DOCX | ✅ | Good | Images | Full text indexing OK |
| XLSX | ✅ | Good | Formulas, colors | Tables as Markdown OK |
| PPTX | ✅ | Limited | Slide layout, speaker notes | Text extraction only |
| HTML | ✅ | Good | JS/styles | Semantic Markdown OK |
| TXT | ✅ | Good | None | Direct passthrough |

**Cortex planning**: If document count heavy in {PDF, DOCX, XLSX}, TransmutationLite sufficient. If PPTX or OCR-heavy, recommend full Transmutation.

### Priority 3: Metadata for Faceting

**Cortex can facet by**:
- `format` (PDF, DOCX, XLSX, etc.)
- `pageCount` (1–10, 11–50, 50+)
- `author` (if extracted)
- `createdAt` (date range)
- `fileSize` (size buckets)

**Example Cortex query**:
```sql
SELECT * FROM documents
WHERE format = 'pdf' AND pageCount > 10 AND createdAt > '2025-01-01'
ORDER BY createdAt DESC
```

### Priority 4: Integration Path

**Cortex-compatible pipeline**:

```
Source Docs (file system / S3)
  ↓
TransmutationLite.convertFile(path)
  ↓
ConversionResult { markdown, metadata, warnings }
  ↓
Vectorizer.embed(markdown)
  ↓
Cortex.index({ markdown, metadata, embeddings })
  ↓
Meilisearch (text search)
+ Nexus (graph + vector search)
```

### Priority 5: Performance for Batch Indexing

**Cortex should know**:
- Batch 4 parallel conversions = ~500 ms for 4×2MB PDFs
- Single large PDF (100 pages) = ~1–2 seconds
- PPTX slowest (jszip); may be bottleneck in large batches
- Cache hits are near-instant (<1 ms)

**Cortex planning**:
- Enable caching for repeated documents
- Use `--parallel 4` or `--parallel 8` depending on memory
- Monitor memory (150 MB for 4 parallel conversions)
- Set `maxPages` to limit processing of very long documents

### Priority 6: Error Handling in Cortex

**Expected errors from TransmutationLite**:

```typescript
try {
  const result = await converter.convertFile(path);
} catch (error) {
  if (error instanceof ConversionError) {
    // Log format + cause for debugging
    logger.error(`Conversion failed: ${error.format}, cause: ${error.cause?.message}`);
    // Cortex: mark document as "failed" + store error reason
  }
}
```

**Cortex should track**:
- Documents that failed conversion (and why)
- Unsupported formats (attempt with full Transmutation)
- Path issues (missing files, permissions)

## Cortex Ingestion Workflow

### Recommended Cortex Task

**Task: "Ingest TransmutationLite as document conversion source"**

1. **Classify phase**: Document format detection
2. **Convert phase**: Call TransmutationLite.convertFile() or use CLI
3. **Extract phase**: Pull metadata + content
4. **Index phase**: Store in Meilisearch (text) + Nexus (graph)
5. **Monitor phase**: Track success rate, timing, errors

### Data Model for Cortex Storage

```typescript
// Cortex document index
interface IndexedDocument {
  id: string;
  sourceFormat: DocumentFormat;        // From metadata.format
  sourceSize: number;                  // metadata.fileSize
  sourcePageCount?: number;            // metadata.pageCount
  sourceAuthor?: string;               // metadata.author
  sourceCreatedAt?: Date;              // metadata.createdAt
  markdownContent: string;             // ConversionResult.markdown
  conversionTimeMs: number;            // ConversionResult.conversionTimeMs
  warnings?: string[];                 // ConversionResult.warnings
  embeddings?: number[];               // From Vectorizer
  graphNodeId?: string;                // Nexus graph reference
  indexedAt: Date;                     // Cortex timestamp
}
```

## Knowledge Base Entries for Cortex

### Key Learnings

1. **TransmutationLite is format-agnostic on output**: All formats → Markdown. No special handling per format.
2. **Metadata is optional but valuable**: Always extract and store (enables faceting).
3. **Cache is optional but recommended**: Enable for repeated documents; costs ~memory.
4. **PPTX is slower**: If PPTX-heavy workloads, consider full Transmutation.
5. **No images in Lite**: Plan accordingly; full Transmutation for image-heavy docs.

### Integration Patterns

- **Classify pipeline**: Classify → TransmutationLite → LLM classification
- **Indexing pipeline**: Docs → TransmutationLite → Vectorizer → Cortex → Meilisearch + Nexus
- **Fallback**: TransmutationLite fails → try full Transmutation (if available)

## Related Cortex Features

### Cortex Consolidator

- May consume TransmutationLite output
- Should handle ConversionResult format
- Needs to store metadata alongside content

### Cortex Sentinels

- Monitor document ingestion success rate
- Alert if conversion failures exceed threshold
- Track performance (avg conversion time)

### Cortex Dashboard

- Display format distribution (PDF %, DOCX %, etc.)
- Show pageCount histogram
- Surface conversion errors by format

## Future Enhancement Ideas

1. **Streaming conversion**: For very large batches (>1K documents)
2. **Format upgrade path**: If TransmutationLite fails, retry with full Transmutation
3. **Incremental indexing**: Cache results; only re-index changed documents
4. **Format detection by content**: Use magic bytes instead of extension
5. **Parallel vectorization**: Vectorize while TransmutationLite still converting

## Open Questions for Cortex Integration

1. Should Cortex store **original documents** (PDF bytes) or just **markdown**?
2. Should Cortex **retry failed conversions** or mark them permanently failed?
3. Should Cortex **prune images** from Word documents or **preserve styling only**?
4. Should Cortex use **caching** for repeated documents or assume one-time ingestion?
5. Should Cortex **upgrade to full Transmutation** for documents >50 pages?
