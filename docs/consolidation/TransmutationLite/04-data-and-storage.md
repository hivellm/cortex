# TransmutationLite — Data & Storage

## Data Model

### ConversionOptions (Input)

```typescript
interface ConversionOptions {
  // Preserve original formatting where possible (default: true)
  preserveFormatting?: boolean;

  // Extract images (not implemented in lite version; default: false)
  extractImages?: boolean;

  // Maximum page/sheet to process; 0 = all (default: 0)
  maxPages?: number;

  // Custom output format options (format-specific)
  formatOptions?: Record<string, any>;
}
```

### ConversionResult (Output)

```typescript
interface ConversionResult {
  // Converted markdown content (main output)
  markdown: string;

  // Document metadata
  metadata: DocumentMetadata;

  // Conversion time in milliseconds
  conversionTimeMs: number;

  // Any warnings during conversion
  warnings?: string[];
}
```

### DocumentMetadata

```typescript
interface DocumentMetadata {
  // Original file format (PDF, DOCX, XLSX, PPTX, TXT, HTML, UNKNOWN)
  format: DocumentFormat;

  // File size in bytes
  fileSize: number;

  // Number of pages/sheets/slides (if available)
  pageCount?: number;

  // Document title (if available)
  title?: string;

  // Document author (if available)
  author?: string;

  // Creation date (if available)
  createdAt?: Date;

  // Format-specific metadata (producer, sheets, slides, etc.)
  extra?: Record<string, any>;
}
```

## Output Format

### Markdown Conversion Targets

- **PDF**: Plain text extraction with basic formatting
- **DOCX**: Preserves headings, lists, tables; removes images
- **XLSX**: Each sheet becomes a Markdown section with tables
- **PPTX**: Slide titles and text; no layout preservation
- **HTML**: Turndown conversion; clean semantic Markdown
- **TXT**: Normalized text (trailing whitespace trimmed)

### Example Output

**Input:** Excel spreadsheet (2 sheets)
```
Sheet1:
  | Column A | Column B | Column C |
  |----------|----------|----------|
  | Value 1  | Value 2  | Value 3  |

Sheet2:
  | Column X | Column Y |
  |----------|----------|
  | Val A    | Val B    |
```

## Storage Considerations

### Memory Model

- **Files loaded entirely into Buffer** (not streamed)
- **Hard limit: 500 MB** (enforced by validation)
- **Cache overhead**: Configurable (default: 100 entries × average size)

### Cache Implementation

- **Type**: LRU (Least Recently Used) eviction
- **Key**: SHA-256 hash of file content
- **TTL**: Optional max age (configurable)
- **Stats tracked**: Size, hits, memory usage, hit rate

```typescript
const cache = new ConversionCache({ maxSize: 100, maxAge: 3600000 });
cache.set(key, result);
const stats = cache.getStats();
// { size, hits, memoryUsageBytes, hitRate }
```

## Metadata Extraction

### Format-Specific Extraction

| Format | Extracted Metadata | Source |
|--------|-------------------|--------|
| PDF | title, author, producer, pageCount, createdAt | pdf-parse-new |
| DOCX | title, author, createdAt | mammoth |
| XLSX | sheetCount (in extra) | xlsx library |
| PPTX | slideCount (in extra) | jszip metadata |
| HTML | pageTitle (in extra) | HTML head/title |
| TXT | fileSize, format | File system |

### Example Metadata

```typescript
result.metadata = {
  format: DocumentFormat.PDF,
  fileSize: 2048000,
  pageCount: 15,
  title: "Research Paper Title",
  author: "John Doe",
  createdAt: new Date("2025-01-15"),
  extra: {
    producer: "pdfTeX",
    subject: "Machine Learning"
  }
}
```

## Error Handling

### ConversionError

```typescript
class ConversionError extends Error {
  format?: DocumentFormat;  // Format that failed
  cause?: Error;            // Underlying library error
}
```

### Error Categories

1. **Unsupported format**: Format not in supported list
2. **File not found**: Path does not exist
3. **Invalid format**: File corrupted or not matching extension
4. **Buffer too large**: File exceeds 500 MB limit
5. **Path traversal**: Path attempts to escape base directory
6. **Library error**: Underlying library (pdf-parse, mammoth) threw

## Validation Rules

1. **Path traversal protection**: No `..` in paths
2. **Buffer size limit**: 500 MB maximum
3. **Format whitelist**: Only supported formats allowed
4. **File extension matching**: Case-insensitive validation

## Performance Targets

| Format | Typical Size | Estimated Time | Notes |
|--------|--------------|-----------------|-------|
| PDF | 2 MB | 200–500 ms | Text extraction only |
| DOCX | 500 KB | 150–300 ms | Depends on complexity |
| XLSX | 1 MB | 100–200 ms | Table conversion |
| PPTX | 3 MB | 300–600 ms | Basic text extraction |
| HTML | 200 KB | 50–100 ms | Turndown conversion |
| TXT | Any | <50 ms | Normalization only |
