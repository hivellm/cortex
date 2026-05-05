# TransmutationLite — Open Questions & Gaps

## Documentation Gaps

### 1. Real-World Performance Data
**Gap**: Performance estimates are theoretical; no production load testing.

**Current data**:
- Estimated PDF (2 MB): 200–500 ms
- Estimated DOCX (500 KB): 150–300 ms
- Estimated XLSX (1 MB): 100–200 ms

**Missing**:
- Actual performance with 1K+ batch conversions
- Memory profiling under sustained load
- GC (garbage collection) impact with caching enabled
- Performance degradation with maxPages limits

**Action**: Profile with real classification workload (Classify project).

### 2. PPTX Conversion Quality
**Gap**: PPTX support is marked "limited"; no quality assessment.

**Current implementation**: Uses jszip for basic text extraction (no layout).

**Missing**:
- Sample output for reference
- Known loss scenarios (speaker notes, animations, etc.)
- Performance comparison to full Transmutation
- User feedback on usability

**Action**: Generate PPTX test samples; compare output to Transmutation Rust.

### 3. Metadata Extraction Completeness
**Gap**: Format-specific metadata extraction is partial; not all fields extracted.

**Current**:
- PDF: title, author, producer, pageCount, createdAt
- DOCX: title, author, createdAt
- XLSX: sheetCount (in extra)
- PPTX: slideCount (in extra)
- HTML: pageTitle (in extra)
- TXT: fileSize, format

**Missing documentation**:
- How reliable is author extraction? (Some PDFs have no author)
- What if metadata fields are missing?
- How to access `extra` fields in Cortex?

**Action**: Add real-world PDF samples to test suite; document extraction reliability per format.

## Feature Gaps

### 1. Image Handling
**Gap**: Images are not extracted or processed in lite version.

**Current**: Images silently dropped from PDF, DOCX conversions.

**Potential issues**:
- Users may expect image extraction (like full Transmutation)
- Diagram-heavy documents lose information
- No warning in output about image loss

**Considered but deferred**:
- Add `extractImages` option (exists in interface but not implemented)
- Return image metadata in warnings array
- Support for image embedding as base64

**Decision**: Keep deferred until Cortex signals need.

### 2. OCR for Scanned Documents
**Gap**: Scanned PDFs (image-only) are not recognized; conversion fails.

**Current behavior**: pdf-parse-new returns empty string for image-only PDFs.

**Potential fix**:
- Detect image-only PDFs (text content empty)
- Return warning: "Document is image-only; OCR required"
- Optionally call full Transmutation with Tesseract

**Decision**: Deferred; would require Transmutation integration.

### 3. Streaming/Chunking for Large Files
**Gap**: Files are loaded entirely into memory; no streaming support.

**Current limit**: 500 MB hard cap (reasonable for classification).

**Potential enhancement**:
- Stream-based PDF parsing for >100 MB files
- Chunk large XLSX sheets into multiple outputs
- Incremental DOCX parsing

**Decision**: Low priority; maxPages option mitigates for classification use.

### 4. Plugin System for Custom Converters
**Gap**: No way to add custom format converters without modifying core.

**Current**: Fixed set of 6 converters; new formats require PR.

**Potential enhancement**:
- Register custom converters at runtime
- Plugin interface for third-party formats (e.g., ODF, RTF)

**Decision**: Deferred; not needed for initial HiveLLM integration.

## Integration Gaps

### 1. Cortex Integration Not Yet Implemented
**Gap**: TransmutationLite documented but not integrated into Cortex pipeline.

**Missing**:
- Cortex tasks for batch document conversion
- Cortex sentinels for conversion failure monitoring
- Cortex dashboard metrics (format distribution, errors)

**Action**: Phase 11v (Cortex) should address this.

### 2. Full Transmutation Fallback Not Implemented
**Gap**: No automatic fallback if TransmutationLite fails (e.g., OCR-required documents).

**Current**: Conversion fails; caller must handle.

**Potential enhancement**:
- Detect image-only PDFs
- Automatically attempt full Transmutation
- Log fallback events

**Decision**: Requires Transmutation integration; out of scope for lite version.

### 3. Classify Integration Incomplete
**Gap**: TransmutationLite ready but Classify not yet consuming it.

**Current status**: TransmutationLite built; Classify still uses placeholder logic.

**Action**: Classify project must integrate when ready.

## Performance Gaps

### 1. No Streaming Vectorization
**Gap**: Documents converted → stored in memory → then vectorized (serial).

**Current**: ConversionResult in memory; then passed to Vectorizer.

**Potential optimization**:
- Stream Markdown to Vectorizer while converting
- Vectorize in parallel with conversion

**Decision**: Deferred; current sequential approach acceptable for batch sizes <1K.

### 2. Cache Effectiveness Unknown
**Gap**: Cache is built in but no production data on hit rates.

**Assumptions**:
- Repeated classifications of same document → cache hit
- TTL (1 hour default) sufficient for typical workflows

**Missing data**:
- Actual cache hit rates in Classify
- Memory overhead with different cache sizes
- Cache eviction impact on performance

**Action**: Monitor cache stats in production (Classify).

### 3. Parallel Batch Default Suboptimal?
**Gap**: Default 4 parallel conversions may not be optimal for all scenarios.

**Current**: `--parallel 4` hardcoded default.

**Scenarios**:
- Small files (KB): Could do 16+ parallel safely
- Large files (100 MB): 1–2 parallel safer (memory)
- Mixed workloads: Adaptive parallelism?

**Action**: Benchmark with real Classify workload; adjust default if needed.

## Compatibility Gaps

### 1. Node.js Version Support
**Gap**: Requires Node.js ≥18.0.0; drops support for older LTS versions.

**Rationale**: ESM modules, Promise features, modern APIs.

**Potential issue**: Deployments on older Node.js unable to use.

**Action**: Document clearly in README; no plan to backport.

### 2. Windows Path Handling
**Gap**: Path handling may have Windows-specific issues (backslashes, drive letters).

**Current**: Uses native Node.js path module; assumed to be cross-platform.

**Missing**: Explicit Windows testing in CI/CD (GitHub Actions runs on Windows).

**Action**: CI/CD includes Windows; monitor for issues.

## Security Gaps

### 1. Dependency Vulnerabilities
**Gap**: 7 known vulnerabilities in dependencies (6 moderate, 1 high).

**Current status**: Not yet audited for critical impact.

**Dependencies affected**:
- pdf-parse-new, mammoth, xlsx, jszip, turndown, commander

**Action**: Security review required before Cortex production use.

### 2. Path Traversal Detection
**Gap**: Validation checks for `..` in paths but may miss edge cases.

**Current check**: Simple string matching for `..`.

**Potential gap**:
- Symlink attacks (creating symlink outside base dir)
- Windows UNC paths (\\network\share)
- URL-encoded traversal (%2e%2e)

**Action**: Enhance validation; add tests for edge cases.

## Testing Gaps

### 1. Real-World Document Library
**Gap**: Tests use generated fixtures; no large corpus of real documents.

**Current**: Simple DOCX/XLSX/PPTX generated by officegen; arXiv PDFs for PDF tests.

**Missing**:
- Scanned PDFs (image-only)
- Corrupted/malformed documents
- Large documents (100+ MB)
- Mixed-language documents
- Documents with complex formatting

**Action**: Build test corpus as classification workload grows.

### 2. Performance Regression Testing
**Gap**: Benchmarks exist but not integrated into CI/CD.

**Current**: `npm run benchmark` must be run manually.

**Missing**: Automated performance checks on commits.

**Action**: Add performance gate to CI (warn if >10% regression).

### 3. Browser/Node.js Compatibility
**Gap**: Library is Node.js only; no browser support.

**Current**: ESM module; used in Node.js only.

**Assumption**: No browser use case for document conversion (correct).

**Action**: Document clearly; no plan to support browsers.

## Operational Gaps

### 1. Production Monitoring
**Gap**: Metrics API exists but no integration with production monitoring (Datadog, New Relic, etc.).

**Current**: In-memory metrics; no external reporting.

**Missing**:
- Export metrics to monitoring system
- Alert on conversion failures
- Track performance degradation

**Action**: Cortex/Classify responsible for extracting and monitoring metrics.

### 2. Logging Integration
**Gap**: Logger is simple; no integration with structured logging systems.

**Current**: Console output only.

**Missing**:
- JSON structured logging (for log aggregation)
- Log level per converter (some formats verbose)
- Integration with winston, pino, etc.

**Action**: Low priority; can be added if Cortex needs structured logs.

### 3. Versioning & Compatibility
**Gap**: Version 0.6.2 not yet published; breaking changes possible before 1.0.

**Current**: Pre-1.0; no semantic versioning guarantees.

**Risk**: API changes in 0.7.0 may break Cortex integration.

**Action**: Plan 1.0 release once Cortex integration complete + stable.

## Recommendations for Cortex

### High Priority

1. **Security audit** of dependencies (7 vulnerabilities)
2. **Production performance testing** with Classify workload
3. **Cortex integration** (tasks, sentinels, dashboard)

### Medium Priority

1. **Real-world document corpus** for testing
2. **Performance regression CI/CD gate**
3. **Fallback to full Transmutation** for failed conversions

### Low Priority

1. Structured logging integration
2. Plugin system for custom converters
3. Streaming support for very large files

## Timeline Unknowns

- **npm publication**: Ready but awaiting approval
- **Classify integration**: Planned but not scheduled
- **Full Transmutation integration**: No current timeline
- **Version 1.0**: Post-integration; no date set
