# TransmutationLite — Decisions & Rationale

## Why a "Lite" Variant?

### Problem Statement

HiveLLM Classify needs document conversion for classification tasks. The full Transmutation Rust library excels at production RAG systems but:

1. **Requires Rust toolchain**: Extra complexity for Node.js-only projects
2. **Over-engineered for classification**: OCR, audio, video unnecessary
3. **Slower integration**: FFI or subprocess overhead vs. direct library
4. **Not Node.js native**: Classify is TypeScript; mixing Rust complicates architecture

### Solution: TransmutationLite

**TypeScript library** optimized for:
- Easy npm integration (one dependency)
- Node.js native (no subprocess/FFI)
- Fast prototyping and development
- Sufficient quality for classification ("good enough" > perfect)

### Trade-offs Accepted

| Aspect | TransmutationLite | Transmutation |
|--------|-------------------|---------------|
| **Precision** | ⚠️ Basic | ✅ 80%+ |
| **Performance** | ⚠️ Moderate | ✅ 98x faster |
| **Features** | ⚠️ Core only | ✅ Advanced |
| **Setup** | ✅ Simple | ⚠️ Complex |
| **Integration** | ✅ Native | ⚠️ CLI/FFI |

## What Was Kept (Core)

### Supported Formats
- PDF (text-only extraction)
- DOCX (full formatting)
- XLSX (as Markdown tables)
- PPTX (basic text)
- HTML (clean conversion)
- TXT (normalization)

**Rationale**: Common formats cover 95% of classification use cases.

### Library Features
- ✅ **Format detection**: From file extension
- ✅ **Metadata extraction**: Title, author, pageCount, createdAt
- ✅ **Error handling**: Clear ConversionError with cause
- ✅ **Caching**: LRU with SHA-256 hashing (optional)
- ✅ **Logging**: Configurable levels (DEBUG, INFO, WARN, ERROR)
- ✅ **Metrics**: Success rate, timing, cache stats
- ✅ **Validation**: Path traversal protection, 500 MB limit
- ✅ **Batch processing**: Parallel conversions with configurable parallelism
- ✅ **CLI**: Single-file and batch commands
- ✅ **Type safety**: Full TypeScript with strict mode

## What Was Dropped

### Advanced Features (Not in Lite)

| Feature | Why Dropped |
|---------|-----------|
| **OCR** | Requires Tesseract; adds 100+ MB; not needed for text-heavy docs |
| **Audio/Video** | Requires Whisper; out of scope for document conversion |
| **Archives** | ZIP/TAR not common in classification pipelines |
| **Image Extraction** | Classification doesn't require images; PDF text sufficient |
| **High Precision** | Classification tolerates basic quality; trade-off acceptable |
| **Streaming** | Classification docs typically <100 MB; load-all-at-once acceptable |
| **Multiple Output Formats** | Markdown only; sufficient for Classify + Vectorizer |

### Design Simplifications

1. **No plugin system**: Fixed set of converters; extensibility via PR
2. **No custom output directives**: Single Markdown target format
3. **No progress events**: Batch operations log summaries; real-time updates not needed
4. **No library detection**: Magic bytes skipped; extension-based detection sufficient

## Architecture Decisions

### 1. Converter Pattern (Isolated Format Handlers)

**Decision**: Each format has its own converter class implementing `IConverter`.

**Rationale**:
- **Maintainability**: Update one format without touching others
- **Testability**: Each converter tested independently
- **Extensibility**: New formats added without core changes
- **Clear responsibility**: Separation of concerns

### 2. Cache at Converter Level

**Decision**: SHA-256 content hashing for cache keys; LRU eviction.

**Rationale**:
- **Correctness**: Content-based keys prevent stale cache hits
- **Memory efficiency**: LRU keeps hot data, evicts cold
- **Optional**: Caching not forced; can be disabled for tests
- **Observable**: Stats API allows monitoring

### 3. No Streaming

**Decision**: Files loaded entirely into Buffer.

**Rationale**:
- **Simplicity**: No async generator complexity
- **Library compatibility**: All 6 libraries expect full buffers
- **Trade-off acceptable**: Classification docs <100 MB typical
- **Mitigation**: `maxPages` option limits processing per format

### 4. Validation-First Approach

**Decision**: Strict input validation (path traversal, buffer limits).

**Rationale**:
- **Security**: Path traversal attacks prevented
- **Reliability**: Buffer overflow protection (500 MB limit)
- **User feedback**: Clear error messages
- **Production-ready**: Proper error handling

## Performance Decisions

### 1. Batch Parallelism (Default: 4 concurrent)

**Decision**: Configurable parallel conversions in batch mode.

**Rationale**:
- **Throughput**: 4 conversions in parallel ≈ 4x faster than serial
- **Memory**: 4 concurrent × average file size = reasonable overhead
- **Tunable**: `--parallel 8` for high-throughput setups
- **Trade-off**: More parallelism = more memory

### 2. No Result Streaming to Disk

**Decision**: Batch results collected in memory, then written.

**Rationale**:
- **Simplicity**: No file handle management
- **Batch atomic**: All or nothing; no partial results
- **Acceptable**: Classification batches <1K files typical
- **Observable**: Summary statistics logged

## Testing Strategy

### Test Coverage

**177 tests** covering:
- Unit tests per format converter (all 6 formats)
- Integration tests (detection, routing, batch)
- CLI tests (commands, options)
- Cache tests (LRU, expiration, stats)
- Validation tests (path traversal, buffer limits)
- Real-world tests (arXiv PDFs, Office fixtures)

**Rationale**: 100% passing tests on v0.6.1; confidence for production use.

## Version Strategy

- **Current**: v0.6.2 (production-ready)
- **Semantic versioning**: MAJOR.MINOR.PATCH
- **npm publication**: Ready but not yet published
- **Stability**: No breaking changes anticipated in near term

## Documentation Strategy

- **README.md**: Quick start + API overview
- **ARCHITECTURE.md**: System design, data flow
- **API.md**: Full type reference
- **5 examples**: Common usage patterns (basic, detection, batch, advanced, errors)
- **CI/CD ready**: GitHub Actions workflows documented
