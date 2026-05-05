# Transmutation — Decisions & Rationale

## Decision 1: Pure Rust Core (No Python)

**Decision:** Core 8 formats (PDF, DOCX, XLSX, PPTX, HTML, XML, TXT, ZIP) are 100% pure Rust. No Python runtime, no PyO3 dependency, no ML models bundled.

**Rationale:**
- **Docling Constraint:** Original Docling requires Python 3.9+, ~2–3GB of ML models, 5–10s startup
- **Transmutation Goal:** Single binary, instant startup, zero dependencies
- **Trade-off:** 80% text similarity (Fast) vs 100% (Docling with ML)
- **Benchmark Validation:** 98x faster offset acceptable quality loss for high-volume ingestion

**Implications:**
- Library users don't inherit Python dependency
- Container images stay small (<100MB)
- Deployment simplified (no environment configuration)
- FFI mode (future) adds C++ docling-parse optionally for 95%+ quality without Python

**Status:** Final. No Python planned for core formats.

## Decision 2: Feature-Gated Optional Tools

**Decision:** OCR (Tesseract), ASR (Whisper), video (FFmpeg), extended archives (TAR, 7Z) are **feature flags**, not bundled.

**Rationale:**
- Core users (text extraction) don't pay for ML tool overhead
- Advanced users opt-in to specific capabilities
- Build time is proportional to needed features
- `build.rs` provides clear guidance on missing tools

**Implications:**
- `cargo build --features office` gives user 8 core formats in seconds
- `cargo build --features office,image-ocr` pulls Tesseract, adds build time
- CLI binary can bundle all features; library crate can cherry-pick

**Status:** Stable since v0.1.0. Working well.

## Decision 3: No HiveLLM Service Dependencies

**Decision:** Transmutation imports **zero** crates from Vectorizer, Nexus, Synap, Lexum, Expert, or Cortex SDKs.

**Rationale:**
- Transmutation is a **converter**, not a vector index or graph client
- Coupling would force version synchronization with 6+ services
- Transmutation's output (Markdown, JSON) is service-agnostic
- Integration is Cortex's responsibility (orchestration layer)

**Implications:**
- Transmutation can evolve independently (stable interface: input file + options → Markdown)
- Cortex workers call Transmutation CLI or link library; no bidirectional coupling
- Future language bindings (Python, JS) are simpler (no internal HiveLLM SDKs to port)
- Clear ownership: Transmutation team owns format conversion, Cortex team owns ingestion workflow

**Status:** Enforced by architecture. No exceptions planned.

## Decision 4: Markdown as Canonical Output

**Decision:** All format conversions target Markdown as intermediate representation. JSON, CSV, images are derived from Markdown state.

**Rationale:**
- **Universal Format:** Readable by humans and LLMs; widely supported downstream
- **Semantic Preservation:** Headings, lists, tables, code blocks map naturally
- **LLM Optimization:** Markdown is chunked naturally (by heading, paragraph, table)
- **Text Content:** Markdown preserves narrative flow better than raw text

**Implications:**
- Code paths converge on Markdown (less maintenance)
- JSON is Markdown + metadata (not separate extraction)
- OCR and ASR output to Markdown (consistent)
- Integration with Vectorizer expects Markdown or JSON, not binary formats

**Status:** Stable since v0.1.0. No changes planned.

## Decision 5: Memory Optimization (v0.3.0+)

**Decision:** After reports of high memory usage with large PDFs, v0.3.0 implemented:
- Cached regex patterns (11 total, compiled once)
- Pre-allocated buffers
- O(n) page extraction (was O(n²))
- Early PDF byte release

**Rationale:**
- Library users (cortex-workers) process 100s of documents in sequence
- v0.2.x would accumulate memory per document, causing GC pauses
- Caching regex prevents redundant compilation (measurable overhead for per-doc conversions)

**Implications:**
- Peak memory usage down to ~50–100MB for typical 100-page PDFs (was 500MB+)
- Faster batch processing with constant memory footprint
- v0.3.2+ recommended for library usage (Cortex should pin ≥0.3.2)

**Status:** Validated in field (cortex-consolidator). Keep optimizations going forward.

## Decision 6: FFI Mode for High Quality (Future)

**Decision:** To reach 95%+ similarity without Python, implement optional C++ FFI to docling-parse.

**Current Status:** Design phase. Not yet available.

**Rationale:**
- docling-parse is pure C++ (no Python runtime)
- ONNX runtime for layout detection is available C/C++
- FFI avoids PyO3 complexity; users link C++ library
- Precision mode (`--precision`) provides intermediate 77% quality in pure Rust

**Implications:**
- Linux/WSL only initially (docling-parse is research code, minimal Win32 support)
- Optional build step (`build_cpp.sh`)
- When done: `--ffi` flag for max quality; `--precision` for pure Rust balance

**Status:** Planned for v0.4.0. Design in `docs/FFI.md`.

## Decision 7: Windows MSI Distribution

**Decision:** Transmutation provides Windows MSI installer (WiX), not just Cargo.

**Rationale:**
- Windows users expect `.msi` installer
- Removes Rust installation requirement
- Includes dependency checks (Tesseract, Whisper, etc.)
- Professional distribution

**Implications:**
- Release process includes WiX build step
- `docs/MSI_BUILD.md` and `docs/MSI_DEPENDENCIES.md` maintained
- Icon and branding embedded
- Binary is larger (~5MB) but still negligible vs Docling (~2GB)

**Status:** Implemented in v0.1.1+. Maintained in releases.

## Decision 8: Batch Processing with Tokio

**Decision:** `BatchProcessor` uses Tokio async, not Rayon threads (despite Rayon being in dependencies).

**Rationale:**
- I/O-bound (disk read, network future writes)
- Tokio integrates with Cortex's async runtime (tokio-based)
- Configurable parallelism without thread pool complexity
- Better resource sharing in containerized environments

**Implications:**
- `#[tokio::main]` required in CLI
- Library users must be in async context or use `block_on`
- Scales better with Cortex's worker model (async/await)

**Status:** Current architecture. No plans to change.

## Decision 9: Version Pinning for Cortex

**Recommendation:** Cortex should pin `transmutation ≥ 0.3.2` in its Cargo.toml.

**Rationale:**
- v0.3.2 fixes Windows library dependency issues
- v0.3.0+ includes memory optimizations (critical for workers)
- v0.2.x has known memory leaks in large PDF batches
- Semantic versioning: 0.3.z is API-stable

**Implications:**
- Cortex can safely auto-update within 0.3.z range
- Major version bumps (1.0.0) may require code changes
- No need to pin exact version; accept patch releases

**Status:** Policy recommendation for Cortex maintainers.
