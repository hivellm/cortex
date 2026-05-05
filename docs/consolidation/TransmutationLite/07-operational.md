# TransmutationLite — Operational

## Installation

### npm Installation (Recommended)

```bash
npm install @hivehub/transmutation-lite
```

### Local Installation (Monorepo)

```bash
# From within HiveLLM monorepo
npm install

# Uses: "file:../transmutation-lite" dependency (if configured)
```

### Version Requirements

- **Node.js**: ≥18.0.0 (no older LTS versions)
- **npm**: ≥9.0.0 (supports ESM)

## Build & Development

### Build

```bash
npm run build
```

**Output**: `dist/` directory with compiled JavaScript + type definitions
- `dist/index.js` (15 KB) — Main library export
- `dist/index.d.ts` (3.4 KB) — Type definitions
- `dist/cli.js` (19.6 KB) — CLI entrypoint
- `*.js.map` (source maps)

### Development Watch Mode

```bash
npm run dev
```

Watches `src/` for changes and rebuilds continuously.

## Testing

### Run Tests

```bash
npm test
```

- **Test runner**: vitest
- **Coverage**: @vitest/coverage-v8
- **Current**: 177/177 tests passing (100%)

### Run Tests with Coverage

```bash
npm run test:coverage
```

### Watch Mode Tests

```bash
npm run test:watch
```

Continuous test run as files change.

## Code Quality

### Linting

```bash
npm run lint          # Check for linting errors
npm run lint:fix      # Auto-fix linting issues
```

**Linter**: ESLint 9.19.0 with TypeScript support

### Type Checking

```bash
npm run type-check
```

**Checker**: TypeScript compiler (tsc)
- Strict mode enabled
- All files checked
- No `@ts-ignore` allowed

### Code Formatting

```bash
npm run format
```

**Formatter**: Prettier 3.4.2

## CLI Usage

### Available Commands

```bash
# Single file conversion
transmutation-lite convert document.pdf -o output.md
transmutation-lite convert report.docx --max-pages 5

# Batch conversion
transmutation-lite batch ./documents -o ./output --recursive
transmutation-lite batch ./pdfs --parallel 8

# List supported formats
transmutation-lite formats
```

### Command-Line Options

#### `convert <file>`

```bash
transmutation-lite convert <file> [options]

Options:
  -o, --output <path>           Output file path (default: <filename>.md)
  -m, --max-pages <number>      Maximum pages/sheets to process
  --no-preserve-formatting      Disable formatting preservation
  -h, --help                    Show help
```

#### `batch <directory>`

```bash
transmutation-lite batch <directory> [options]

Options:
  -o, --output <path>           Output directory (default: <dir>/output)
  -r, --recursive               Process subdirectories recursively
  -m, --max-pages <number>      Maximum pages/sheets per file
  --parallel <number>           Parallel conversions (default: 4)
  --no-preserve-formatting      Disable formatting preservation
  -h, --help                    Show help
```

#### `formats`

```bash
transmutation-lite formats
```

Outputs list of supported formats (PDF, DOCX, XLSX, PPTX, HTML, TXT).

## Environment Variables

### No Required Environment Variables

TransmutationLite requires no environment configuration. All behavior is controllable via:
- Constructor options (when used as library)
- Command-line arguments (CLI)

### Optional: Node.js Environment

```bash
# Enable detailed debug logging
NODE_DEBUG=transmutation-lite node app.js
```

## Benchmarking

### Run Performance Benchmarks

```bash
npm run benchmark
```

**Output**: Throughput (MB/s), timing, memory usage per format

### Compare Benchmarks

```bash
npm run benchmark:compare
```

Runs comparison against baseline (tracks performance regressions).

## Monitoring & Metrics

### In-Library Metrics

```typescript
const converter = new Converter({ collectMetrics: true });
const metrics = converter.getMetricsSummary();

console.log({
  successRate: metrics.successRate,      // 0.0–1.0
  cacheHitRate: metrics.cacheHitRate,    // 0.0–1.0
  avgConversionTimeMs: metrics.avgConversionTimeMs
});
```

### Logging

```typescript
import { Logger, LogLevel } from '@hivehub/transmutation-lite';

const logger = new Logger({ level: LogLevel.INFO });
const converter = new Converter({ logger });
```

**Log levels**: DEBUG, INFO, WARN, ERROR

## Performance Characteristics

| Operation | Time | Memory |
|-----------|------|--------|
| Convert PDF (2 MB, 15 pages) | 200–500 ms | ~50 MB |
| Convert DOCX (500 KB, 20 pages) | 150–300 ms | ~30 MB |
| Convert XLSX (1 MB, 10 sheets) | 100–200 ms | ~40 MB |
| Cache hit (any format) | <1 ms | Negligible |
| Batch 4 files in parallel | ~500 ms (for 4×2MB PDFs) | ~150 MB |

## Limits & Constraints

| Limit | Value | Note |
|-------|-------|------|
| Max file size | 500 MB | Hard limit; validated at input |
| Max cache entries | Configurable | Default: 100; memory-bounded |
| Max page limit | No hard cap | Controlled per-conversion via `maxPages` |
| Max PDF pages (typical) | Unlimited | But performance degrades beyond 100 pages |
| Supported formats | 6 | PDF, DOCX, XLSX, PPTX, HTML, TXT |
| Output format | Markdown | Single target format (no alternatives) |

## Troubleshooting

### Common Issues

| Issue | Solution |
|-------|----------|
| "Format not supported" | Check file extension; ensure it's in supported list (PDF, DOCX, etc.) |
| "Buffer too large" | File exceeds 500 MB; reduce file size or use `maxPages` option |
| "Path traversal attempt" | Path contains `..`; use absolute or relative paths within base directory |
| "ENOENT: no such file" | File path does not exist; verify path and permissions |
| "Out of memory" | Reduce batch parallelism (`--parallel 2`) or file size |
| Slow PPTX conversion | PPTX uses jszip (slower); consider full Transmutation for production |

### Debug Mode

```bash
# Verbose logging
transmutation-lite convert document.pdf --debug

# Or in code
const logger = new Logger({ level: LogLevel.DEBUG });
```

## Updating Dependencies

```bash
npm update                 # Update all to latest within semver
npm audit fix              # Auto-fix security vulnerabilities
npm outdated               # Show outdated packages
```

**Note**: Package.json currently declares 7 vulnerabilities (6 moderate, 1 high) in dependencies. Coordinate with security review before production deployment.

## Publishing (to npm)

### Pre-Publication Checklist

- [x] All tests passing (177/177)
- [x] Type-check clean
- [x] Linting clean
- [x] Build successful
- [x] README complete
- [x] LICENSE file present (MIT)
- [x] package.json configured
- [x] CI/CD workflows ready

### Publish Command

```bash
npm publish --access public
```

**Note**: First publication; tag as `v0.6.2`.

### Verify Installation

```bash
npm install @hivehub/transmutation-lite
```

Then test:
```bash
npx transmutation-lite formats
```
