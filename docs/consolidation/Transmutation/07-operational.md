# Transmutation — Operational

## Docker & Containerization

### Official Docker Image
**Repository:** `hivehub/transmutation` (if published)  
**Current:** Not found on Docker Hub; build locally if needed.

### Build Transmutation Docker Image (Example)
```dockerfile
FROM rust:1.85-slim as builder

WORKDIR /build
COPY . .

# Install optional dependencies
RUN apt-get update && apt-get install -y \
    tesseract-ocr \
    libopencv-dev \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

# Build CLI with all features
RUN cargo build --release --features "office,image-ocr,audio,video,cli"

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/transmutation /usr/local/bin/

# Runtime dependencies
RUN apt-get update && apt-get install -y \
    tesseract-ocr \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/usr/local/bin/transmutation"]
```

### Cortex Container Integration
**Expected:** Transmutation is invoked by `cortex-consolidator` worker, either:
- As CLI subprocess (spawn container or use binary)
- As library (Cortex worker links `transmutation` crate)

**Container Size Expectations:**
- Pure Rust binary: ~5MB
- With Tesseract: +100MB
- With FFmpeg: +200MB
- Total image (minimal Debian + tools): ~400–600MB

## Ports & Network

**Transmutation does NOT expose any network services.**
- No HTTP server
- No gRPC endpoint
- No TCP listening

All access is:
- CLI: stdin/stdout/files
- Library: in-process function calls

**No port configuration needed.**

## Environment Variables

### Runtime Configuration
| Variable | Type | Default | Purpose |
|----------|------|---------|---------|
| `RUST_LOG` | String | `info` | Log level (trace, debug, info, warn, error) |
| `TRANSMUTATION_CACHE_DIR` | Path | `$XDG_CACHE_HOME/transmutation` | Conversion cache location |
| `TRANSMUTATION_TEMP_DIR` | Path | System temp | Temporary file location |
| `TRANSMUTATION_MAX_WORKERS` | Count | CPU count | Max parallel conversions |
| `TRANSMUTATION_TIMEOUT` | Seconds | 300 | Conversion timeout |

### OCR Configuration (if `tesseract` feature enabled)
| Variable | Type | Default | Purpose |
|----------|------|---------|---------|
| `TESSERACT_PATH` | Path | Auto-detect | Tesseract binary location |
| `TESSDATA_PREFIX` | Path | Auto-detect | Tesseract language data directory |

### Audio Configuration (if `audio` feature enabled)
| Variable | Type | Default | Purpose |
|----------|------|---------|---------|
| `WHISPER_MODEL` | String | `base` | OpenAI Whisper model size |

### Example (Cortex worker setup)
```bash
export RUST_LOG=debug
export TRANSMUTATION_MAX_WORKERS=4
export TRANSMUTATION_TIMEOUT=600

transmutation batch /data/input/*.pdf -o /data/output/ --parallel 4
```

## Logging

**Framework:** `tracing` + `tracing-subscriber`  
**Format:** Structured logging (JSON when `--json` flag passed)

**Log Levels:**
- `error`: Conversion failures, critical issues
- `warn`: Missing optional dependencies, fallbacks
- `info`: Conversion summaries, throughput
- `debug`: File type detection, option application
- `trace`: Individual text extraction steps (verbose)

**Output:** stdout (default), configurable via `RUST_LOG`

**Example logs:**
```
2026-02-28T10:15:42.123Z INFO transmutation: Initializing Transmutation v0.3.2
2026-02-28T10:15:42.124Z INFO transmutation::converters::pdf: Converting document.pdf (2.2MB, 15 pages)
2026-02-28T10:15:42.345Z INFO transmutation: Conversion complete: 15 pages, 0.22s, 68.2 pages/sec
```

## Monitoring & Metrics

**Built-in Metrics:**
- Conversion time (per document)
- Pages processed (per document)
- Throughput (pages/sec)
- Memory usage (tracked by OS, not instrumented)
- Error rate (failures per batch)

**For Cortex Integration:**
- Emit logs with `@timestamp`, `document_id`, `status`, `duration_ms`
- Forward to centralized logging (if available)
- Track p95 latency per format

**Future:** OpenTelemetry support (optional feature, not planned for v0.3.x).

## Dependency Management

### Build-Time Dependencies
| Tool | Status | Install | Required For |
|------|--------|---------|--------------|
| Rust 1.85+ | ✅ Required | [rustup.rs](https://rustup.rs) | Compilation |
| Cargo | ✅ Required | Via rustup | Package management |
| Git | ✅ Required (in CI) | Platform package manager | Source control |

### Runtime Dependencies (Optional)
| Tool | Feature | Status | Install | Version |
|------|---------|--------|---------|---------|
| **Tesseract** | `tesseract` | Optional | `apt-get install tesseract-ocr` | 4.0+ |
| **Whisper CLI** | `audio`, `video` | Optional | Via pip or binary | Latest |
| **FFmpeg** | `video` | Optional | `apt-get install ffmpeg` | 4.0+ |
| **poppler-utils** | `pdf-to-image` | Optional | `apt-get install poppler-utils` | 0.86+ |
| **LibreOffice** | Office image extraction | Optional | `apt-get install libreoffice` | 7.0+ |

**Detection:** `build.rs` auto-detects and provides guidance if missing.

**Installation Scripts:**
- Linux: `./install/install-deps-linux.sh`
- macOS: `./install/install-deps-macos.sh`
- Windows: `.\install\install-deps-windows.ps1`

## Performance Tuning

### For Cortex Workers
**Scenario:** Processing 1000 PDFs in sequence.

**Recommended Config:**
```rust
ConverterConfig {
    enable_cache: true,           // Avoid re-processing identical docs
    max_parallel: 4,              // Match worker count (not CPU count)
    timeout: Duration::from_secs(600),  // 10 min for large docs
}
```

**Batch Processing:**
```bash
transmutation batch /data/*.pdf -o /data/out/ --parallel 4 --progress
```

**Tuning Tips:**
- Increase `--parallel` if I/O is bottleneck (disk is fast SSD)
- Decrease if CPU is maxed (batch has many large PDFs)
- Enable caching for repeated conversions (same document ingested twice)
- Monitor memory; if >500MB, reduce batch size or add `--split-pages`

### For Large PDFs (>100 pages)
- Use `--split-pages` to output one file per page (helps with chunking)
- Use `--precision` mode for better quality (still 250x faster than Docling)
- Avoid `--precision --extract-images` together (slower, larger output)

## Troubleshooting

### Common Issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| "PDF extraction failed" | Corrupted PDF or unsupported version | Validate PDF with `pdfinfo` or `mutool info` |
| "Tesseract not found" | OCR feature enabled but binary missing | Run `./install/install-deps-<os>.sh` or `apt-get install tesseract-ocr` |
| "Out of memory" | Large batch without split-pages | Add `--split-pages` or reduce `--parallel` |
| "Timeout exceeded" | Document too large or system slow | Increase `TRANSMUTATION_TIMEOUT` or use `--precision` (pure Rust) |
| "UTF-8 boundary panic" | Non-ASCII text near boundary (fixed in v0.3.1+) | Update to v0.3.2+ |

### Debug Mode
```bash
RUST_LOG=debug transmutation convert document.pdf -o output.md
```

### Validation
```bash
# Verify binary
transmutation --version  # Should print 0.3.2

# Verify optional dependencies
which tesseract  # If using OCR
which ffmpeg     # If using video
```

## Release & Deployment

### Current Release
**Version:** 0.3.2  
**Published:** Feb 28, 2026  
**Artifacts:** [GitHub releases](https://github.com/hivellm/transmutation/releases)

### Release Channel
- **crates.io:** `transmutation = "0.3.2"` (library)
- **GitHub Releases:** Pre-built binaries (CLI, Linux/macOS/Windows)
- **Docker Hub:** Not published (use local build)
- **Windows MSI:** Available in releases

### Cortex Recommendation
Cortex should pin in `Cargo.toml`:
```toml
[dependencies]
transmutation = "0.3"  # Allows >=0.3.0, <0.4.0
```

Or explicitly:
```toml
[dependencies]
transmutation = "0.3.2"  # Exact version if critical
```
