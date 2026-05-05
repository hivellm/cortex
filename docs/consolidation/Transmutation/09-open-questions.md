# Transmutation — Open Questions & Gaps

## Technical Gaps

### 1. FFI Mode (C++ docling-parse) Status
**Question:** When will Precision Mode (FFI) be ready for production?

**Current State:** Design phase, planned for v0.4.0  
**Blocker:** C++ library compilation on multiple platforms (Linux, macOS, Windows)

**Known Issues:**
- Windows bindings not yet validated (docling-parse is research code)
- ONNX runtime integration (layout model, table structure model) is partial
- No published pre-built binaries for docling-parse C++ library

**Impact on Cortex:** If 95%+ quality is non-negotiable (e.g., for legal documents), v0.3.2 Precision mode (77%) may be insufficient. Cortex should evaluate trade-off: +15% quality vs 3–5x slower.

**Recommendation:** Monitor roadmap; test alpha builds when available.

---

### 2. Python Bindings (PyO3)
**Question:** Will Transmutation support Python natively?

**Current State:** README mentions Python bindings as "future", not implemented  
**Rationale:** No demand signal from Cortex team; primary use case is Rust library

**Implication:** If Cortex or external services need Python access, they must:
- Call CLI subprocess
- Use Transmutation Rust library via PyO3 wrapper (external, not official)
- Stick with Docling (Python native)

**Recommendation:** Clarify with Transmutation maintainers if Python is strategic.

---

### 3. JavaScript/TypeScript Bindings (Neon)
**Question:** Can GUI (Cortex web dashboard) call Transmutation directly?

**Current State:** README mentions Neon bindings as "future", not implemented  
**Realistic:** Low priority; GUI is usually stateless, conversion happens server-side (Cortex workers)

**Workaround:** GUI calls Cortex API → Cortex workers → Transmutation → result

**Recommendation:** Not worth pursuing unless GUI has offline-first use case.

---

### 4. Streaming / Incremental Output
**Question:** Can Transmutation output results incrementally (e.g., one page at a time) rather than buffering the entire document?

**Current State:** Not supported. `ConversionResult` is a complete object returned after full conversion.

**Impact:** Large PDFs (500+ pages) must be held in memory or split via `--split-pages` (creates N files).

**Use Case:** If Cortex wants to stream results to Vectorizer as they're available (not wait for full conversion), this gap matters.

**Workaround:** `--split-pages` to output one file per page; consolidate in Cortex workers.

**Recommendation:** File feature request if streaming is a bottleneck.

---

## Integration Gaps

### 5. Vectorizer Integration (Official)
**Question:** Should Transmutation be bundled with Vectorizer?

**Current State:** Separate projects; Transmutation is library, Vectorizer is service  
**Decision Needed:** Is Transmutation a **prerequisite** for Vectorizer, or **optional upstream**?

**Option A (Current):** Cortex orchestrates (calls Transmutation, then Vectorizer)  
**Option B (Future):** Vectorizer bundles Transmutation as built-in step

**Impact on Cortex:** Option B would offload responsibility; Option A requires Cortex to manage both.

**Recommendation:** Clarify with Vectorizer maintainers; document decision in `.rulebook/decisions/`.

---

### 6. Storage Integration (Transmutation Cache vs Cortex Cache)
**Question:** Should Transmutation's built-in cache (SHA256-based) be used, or should Cortex implement its own?

**Current State:** Transmutation has optional local cache (in `$XDG_CACHE_HOME`). Cortex could also cache converted Markdown in Archive.

**Trade-offs:**
| Approach | Pros | Cons |
|----------|------|------|
| Transmutation cache | Simple, automatic | Local-only, not queryable |
| Cortex (Archive) cache | Queryable, persistent, centralized | Requires separate storage layer |
| Both | Redundancy, fast local + persistent | Overhead, sync complexity |

**Recommendation:** Use Cortex cache (Archive) for production; Transmutation cache for dev/testing.

---

### 7. Monitoring & SLOs
**Question:** What SLOs should Cortex enforce for Transmutation?

**Currently Undefined:**
- Max conversion time per format
- Acceptable error rate
- Quality gate (% documents with >70% text extraction)
- Throughput targets (docs/sec)

**Recommendation:**
- Set conversion timeout: 5 min per doc (reasonable for most PDFs)
- Error rate SLO: <1% (investigate outliers)
- Quality gate: 85% of sample corpus converts successfully
- Throughput: ≥50 docs/sec with 4 workers

---

## Functional Gaps

### 8. Multi-Language OCR
**Question:** Does Tesseract support all required languages?

**Current State:** Transmutation passes `--lang` parameter to Tesseract; depends on installed language packs  
**Risk:** Chinese, Arabic, Indic scripts need separate training data

**Impact:** If Cortex ingests multilingual documents, OCR may fail silently.

**Recommendation:** Document supported languages; test with representative samples. Provide Docker image with common language packs.

---

### 9. Table Extraction Fidelity
**Question:** How accurately does Transmutation extract tabular data?

**Current State:** Tables are converted to Markdown format; not parsed into structured JSON.

**Impact:** If Cortex needs to search within table cells or preserve table structure exactly, Markdown tables may be insufficient.

**Use Case:** Legal contracts, financial reports often have complex tables.

**Recommendation:** Evaluate with sample documents. If accuracy is poor, consider using FFI mode (planned for v0.4.0) which includes table structure detection.

---

### 10. Header/Footer Removal
**Question:** Does `--remove-headers` / `--remove-footers` work correctly?

**Current State:** Implemented via heuristics (pattern matching for repeated content); not ML-based.

**Risk:** Legitimate content that appears on multiple pages may be incorrectly removed.

**Impact:** If Cortex sees missing content in converted documents, this may be the culprit.

**Recommendation:** Test with document samples; if unreliable, disable (`optimize_for_llm: false` in options).

---

### 11. Layout Preservation
**Question:** Is layout preservation (e.g., indentation, columns) accurate?

**Current State:** Markdown preserves basic structure (headings, lists); column layouts are converted to linear text.

**Impact:** Research papers with multi-column text may become harder to read after conversion.

**Recommendation:** This is an inherent limitation of text-only output. If visual preservation is required, use `--extract-images` or wait for FFI mode.

---

## Operational Gaps

### 12. Windows Support & Testing
**Question:** Is Transmutation fully tested on Windows?

**Current State:** Windows MSI installer exists; unit tests use cross-platform crates  
**Known Risk:** Path handling, line endings, external tool invocation may have Windows-specific bugs

**Recommendation:** Test on Windows 10/11 before production Cortex deployment.

---

### 13. Performance Profiling
**Question:** Where is Transmutation slowest? CPU-bound? I/O-bound?

**Current State:** No detailed profiling data published; "250x faster than Docling" is benchmark-level claim.

**Use Case:** Understanding bottlenecks helps tune Cortex worker config.

**Recommendation:** Run `cargo bench` with `criterion` on representative documents. Profile with `flamegraph` or `perf` if needed.

---

### 14. Container Resource Limits
**Question:** What are safe CPU/memory limits for Transmutation in Kubernetes?

**Current State:** Recommended 50–100MB per conversion; no guidance on CPU throttling.

**Use Case:** Cortex workers run in containers with resource limits.

**Recommendation:** Document empirically (measure actual usage with `--split-pages` and batch processing). Suggest defaults: CPU=1 core/worker, Memory=512MB.

---

## Documentation Gaps

### 15. Example Integrations
**Question:** Are there example projects showing Transmutation + Cortex integration?

**Current State:** No official examples; only README code snippets.

**Recommendation:** Create example in `.rulebook/tasks/` or separate repo showing:
- cortex-consolidator calling Transmutation library
- Error handling and retry logic
- Metrics emission
- Integration tests

---

## Strategic Questions

### 16. Docling Competition
**Question:** How long is Transmutation maintained as Docling alternative?

**Current State:** Active development (v0.3.2 in Feb 2026); roadmap includes v0.4.0 with FFI.

**Assumption:** Transmutation is strategic for HiveLLM (Cortex depends on it). If Docling dramatically improves (faster, pure Rust), this could change.

**Recommendation:** Monitor Docling releases; maintain decision record if switching is considered.

---

### 17. External Contribution Policy
**Question:** Does Transmutation accept external contributions?

**Current State:** Open source (MIT), GitHub repo public; CONTRIBUTING.md exists.

**Implication:** Cortex can propose feature requests or contribute fixes directly.

**Recommendation:** File issues for any bugs found during Cortex integration; offer to contribute fixes if maintainers are willing.
