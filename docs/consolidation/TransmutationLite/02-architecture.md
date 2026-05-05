# TransmutationLite — Architecture

## System Diagram

```
┌─────────────────────────────────────────────────────────┐
│                  Transmutation Lite                      │
├─────────────────────────────────────────────────────────┤
│                                                         │
│   CLI (commander)   ──────▶   Converter Class           │
│                                     │                    │
│                                     ▼                    │
│                           Format Detection              │
│                                     │                    │
│              ┌──────────────────────┴──────────────────┐ │
│              ▼                                         ▼ │
│        Buffer Converter                       File Converter│
│              │                                       │    │
│              └──────────────┬──────────────────────┬──┘   │
│                             ▼                      │      │
│                    Converter Router ──────────────┤      │
│                             │                     │      │
│        ┌────────┬───────┬───┴──┬───────┬────────┐│      │
│        ▼        ▼       ▼      ▼       ▼        ▼│      │
│     PDF   DOCX  XLSX  PPTX   HTML    TXT        │      │
│     Conv  Conv  Conv  Conv   Conv    Conv       │      │
│        │    │    │     │      │       │         │      │
│        └────┴────┴─────┴──────┴───────┴─────────┘      │
│                        │                               │
│                        ▼                               │
│         External Libraries (6 deps)                   │
│  pdf-parse, mammoth, xlsx, jszip, turndown           │
│                        │                               │
│                        ▼                               │
│    Markdown Output + DocumentMetadata                │
│                                                        │
└────────────────────────────────────────────────────────┘
```

## Core Modules

### 1. **Converter Class** (`src/index.ts`)
- Orchestrates all format converters
- Performs format detection from file extension
- Routes to appropriate converter
- Manages caching, logging, metrics

### 2. **Format Converters** (`src/converters/`)
- **PdfConverter**: Uses `pdf-parse-new` for text extraction
- **DocxConverter**: Uses `mammoth` for Word documents
- **XlsxConverter**: Uses `xlsx` library (converts to Markdown tables)
- **PptxConverter**: Uses `jszip` for basic text extraction
- **HtmlConverter**: Uses `turndown` for clean conversion
- **TxtConverter**: Native normalization

All implement `IConverter` interface (convert, getFormat, canHandle).

### 3. **Type System** (`src/types.ts`)
```typescript
enum DocumentFormat { PDF, DOCX, XLSX, PPTX, TXT, HTML, UNKNOWN }
interface ConversionOptions { preserveFormatting?, maxPages?, formatOptions? }
interface ConversionResult { markdown, metadata, conversionTimeMs, warnings? }
interface DocumentMetadata { format, fileSize, pageCount?, title?, author?, createdAt?, extra? }
class ConversionError extends Error { format?, cause? }
```

### 4. **Support Modules**
- **Cache** (`src/cache.ts`): LRU cache with SHA-256 content hashing
- **Logger** (`src/logger.ts`): Configurable logging (DEBUG, INFO, WARN, ERROR)
- **Metrics** (`src/metrics.ts`): Success rate, timing, cache stats
- **Validation** (`src/validation.ts`): Path traversal protection, buffer limits (500MB)

### 5. **CLI** (`src/cli.ts`)
- Commands: `convert`, `batch`, `formats`
- Uses `commander` for argument parsing
- Supports parallel batch conversion (default: 4 parallel)

## Data Flow

### Single File Conversion
1. User calls `converter.convertFile(path)`
2. Format detected from file extension
3. File read into Buffer
4. Appropriate converter selected
5. Buffer passed to converter.convert()
6. External library processes buffer
7. Result formatted with metadata
8. Returns ConversionResult

### Batch Processing
1. Directory scanned for supported files
2. Files grouped into parallel batches (default: 4)
3. Each batch: parallel conversions → results collected
4. Summary statistics logged

## Dependencies

### Production (6)
- pdf-parse-new (^1.4.1)
- mammoth (^1.11.0)
- xlsx (^0.18.5)
- jszip (^3.10.1)
- turndown (^7.2.2)
- commander (^14.0.2)

### Development (10)
- TypeScript, tsup, vitest, ESLint, Prettier
- Type definitions (@types/node, @types/pdf-parse, @types/turndown)
- Test helpers (happy-dom, officegen, @vitest/coverage-v8)

## Key Design Decisions

1. **Format Converters are isolated**: Each handles one format; easy to test/update independently
2. **IConverter interface**: Enables pluggability (new formats can be added without core changes)
3. **Caching at Converter level**: SHA-256 content hashing ensures cache validity
4. **No streaming**: Files loaded entirely into memory (limitation: 500MB max)
5. **Validation is strict**: Path traversal protection, buffer limits enforced
