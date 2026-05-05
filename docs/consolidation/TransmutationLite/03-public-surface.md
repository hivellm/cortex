# TransmutationLite — Public Surface

## Library API

### Main Exports

```typescript
// Convenience functions (quick usage)
export async function convert(
  filePath: string,
  options?: ConversionOptions
): Promise<ConversionResult>

export async function convertBuffer(
  buffer: Buffer,
  format: DocumentFormat,
  options?: ConversionOptions
): Promise<ConversionResult>

// Main class
export class Converter {
  constructor(options?: ConverterOptions)
  convertFile(filePath: string, options?: ConversionOptions): Promise<ConversionResult>
  convertBuffer(buffer: Buffer, format: DocumentFormat, options?: ConversionOptions): Promise<ConversionResult>
  detectFormat(filePath: string): DocumentFormat
  isSupported(filePath: string): boolean
  getSupportedFormats(): DocumentFormat[]
  getMetricsSummary(): MetricsSummary
}

// Types
export enum DocumentFormat { PDF, DOCX, XLSX, PPTX, TXT, HTML, UNKNOWN }
export interface ConversionOptions { preserveFormatting?, extractImages?, maxPages?, formatOptions? }
export interface ConversionResult { markdown, metadata, conversionTimeMs, warnings? }
export interface DocumentMetadata { format, fileSize, pageCount?, title?, author?, createdAt?, extra? }
export class ConversionError extends Error { format?, cause? }

// Utilities
export class Logger { constructor(options?: LoggerOptions) }
export enum LogLevel { DEBUG, INFO, WARN, ERROR }
export class ConversionCache { get(), set(), has(), clear(), getStats() }
export interface MetricsSummary { successRate, cacheHitRate, avgConversionTimeMs }
```

### npm Package

```json
{
  "name": "@hivehub/transmutation-lite",
  "version": "0.6.2",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "bin": { "transmutation-lite": "./dist/cli.js" },
  "exports": { ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" } }
}
```

## CLI Commands

### `transmutation-lite convert <file>`

Convert a single file to Markdown.

**Options:**
- `-o, --output <path>` — Output file path (default: `<filename>.md`)
- `-m, --max-pages <number>` — Maximum pages/sheets to process
- `--no-preserve-formatting` — Disable formatting preservation

**Example:**
```bash
transmutation-lite convert document.pdf -o output.md
transmutation-lite convert large.pdf --max-pages 10
```

### `transmutation-lite batch <directory>`

Convert all supported files in a directory.

**Options:**
- `-o, --output <path>` — Output directory (default: `<directory>/output`)
- `-r, --recursive` — Process subdirectories recursively
- `-m, --max-pages <number>` — Maximum pages/sheets
- `--parallel <number>` — Number of parallel conversions (default: 4)
- `--no-preserve-formatting` — Disable formatting

**Example:**
```bash
transmutation-lite batch ./documents -o ./markdown --recursive
transmutation-lite batch ./pdfs --parallel 8 --max-pages 5
```

### `transmutation-lite formats`

List all supported file formats.

## Installation & Usage

### npm Installation
```bash
npm install @hivehub/transmutation-lite
```

### Library Usage (TypeScript)
```typescript
import { Converter, DocumentFormat, LogLevel, Logger } from '@hivehub/transmutation-lite';

const converter = new Converter({
  enableCache: true,
  cacheSize: 100,
  logger: new Logger({ level: LogLevel.INFO }),
  collectMetrics: true,
});

const result = await converter.convertFile('./document.pdf', {
  preserveFormatting: true,
  maxPages: 10,
});

console.log(result.markdown);
console.log('Pages:', result.metadata.pageCount);
console.log('Time:', result.conversionTimeMs, 'ms');
console.log('Cache hit rate:', converter.getMetricsSummary().cacheHitRate);
```

### Library Usage (Convenience Function)
```typescript
import { convert } from '@hivehub/transmutation-lite';

const result = await convert('./document.docx');
console.log(result.markdown);
```

## Format Support Matrix

| Format | Ext | Library | Quality | Notes |
|--------|-----|---------|---------|-------|
| PDF | .pdf | pdf-parse-new | Basic | Text only; no images/formatting |
| DOCX | .docx | mammoth | Good | Full formatting support |
| XLSX | .xlsx, .xls | xlsx | Good | Converts to Markdown tables |
| PPTX | .pptx, .ppt | jszip | Limited | Basic text extraction only |
| HTML | .html, .htm | turndown | Good | Clean Markdown conversion |
| TXT | .txt, .md | native | Good | Direct text handling |
