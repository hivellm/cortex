# Tml — Operational & Build

## Building Tml Compiler

### Prerequisites

| Component | Version | Purpose |
|-----------|---------|---------|
| Zig | 0.15.2 (NOT 0.16+) | C/C++ compiler toolchain |
| CMake | 3.20+ | Build configuration |
| Ninja | Latest | Build executor |
| LLVM | 15+ | Codegen backend |

**Platform Support**:
- Windows: MSVC 19.30+ or Zig CC
- Linux: Zig CC or system Clang 15+
- macOS: Zig CC or system Clang

### First-Time Bootstrap

```bash
# One-time: build vendored LLVM submodule
git submodule update --init --recursive src/llvm-project
scripts\build-llvm.bat         # Windows (30–90 min)
./scripts/build-llvm.sh        # Linux/macOS
```

### Regular Builds

```bash
# Autodetects best compiler (Zig > MSVC > Clang)
scripts\build.bat              # Debug
scripts\build.bat release      # Release

# Force specific compiler
scripts\build.bat --zig        # Zig CC (fastest)
scripts\build.bat --msvc       # MSVC cl.exe
scripts\build.bat --clang      # System Clang

# Other options
scripts\build.bat --clean      # Clean rebuild
scripts\build.bat --tests      # Build C++ unit tests
scripts\build.bat --target tml # Build only tml.exe
```

**Performance**:
- First build: 5–15 minutes
- Cached rebuild: 1–3 seconds

## Running Tests

### Full Suite

```bash
tml test                       # All tests (~8s cached)
tml test --no-cache           # Full rebuild + test (~43s)
tml test --coverage           # Coverage report
tml test --profile            # Tracy profiler output
```

### Targeted Testing

```bash
tml test --suite=core/str          # One module
tml test --filter=json_parse       # Test name pattern
tml test --coverage --profile      # Instrumentation + profiling
```

### Output

NDJSON protocol streaming results:

```json
{"type": "test_start", "name": "test_list_append"}
{"type": "test_pass", "name": "test_list_append", "duration_ms": 2}
{"type": "test_summary", "passed": 12000, "failed": 0}
```

## Performance Metrics

| Metric | Value | Notes |
|--------|-------|-------|
| **Full test suite** | 8s (cached) / 43s (clean) | 12,000+ tests, NDJSON |
| **Cached `tml check`** | 0.68s | With query cache |
| **`tml daemon`** | 22ms rebuild | Work-stealing executor |
| **HTTP server** | 183K req/s | Single machine |
| **Binary size** | 42 KB (hello world) | 3× smaller than Rust |

## Ports & Configuration

### No Default Network Ports

TML compiler doesn't listen on network ports by default. However:

- **HTTP server apps** (in stdlib) default to port 8080
- **Async runtime**: Uses system IOCP (Windows) or epoll (Linux)
- **MCP server**: Runs on stdio (no network port)

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `TML_COVERAGE_FILE` | Path to coverage output |
| `TRACY_PORT` | Tracy profiler connection (default 8086) |
| `TML_DEBUG` | Verbose compiler output |
| `TML_DAEMON_PORT` | Daemon socket (internal, platform-specific) |

### CMake Configuration

Main knobs in `CMakeLists.txt`:

```cmake
set(TML_BACKEND "LLVM" CACHE STRING "LLVM or Cranelift")
set(TML_ENABLE_MCP ON CACHE BOOL "Enable MCP server")
set(TML_ENABLE_TRACY ON CACHE BOOL "Enable Tracy profiler")
set(TML_ENABLE_SANITIZERS OFF CACHE BOOL "ASAN/UBSAN")
```

## Artifact Locations

| Artifact | Location | Purpose |
|----------|----------|---------|
| **Compiler** | `build/debug/tml.exe` or `build/release/tml.exe` | Main executable |
| **LLVM** | `build/llvm/` | Static libraries + includes |
| **Tests** | Output as NDJSON to stdout | Subprocess results |
| **Cache** | `.tml/` in project root | Query cache (fingerprints, LLVM IR) |
| **Docs** | Generated to stdout in JSON/HTML/MD | From `tml doc` |
| **Profiling** | Chrome DevTools format (`.json`) | From `--profile` flag |

## CI/CD Integration

No built-in CI; expected pattern for Cortex:

```yaml
# Dockerfile (example)
FROM ubuntu:22.04
RUN apt install -y zig ninja cmake
WORKDIR /tml
COPY . .
RUN scripts/build-llvm.sh && scripts/build.sh release
RUN tml test --coverage
```

Then use `mcp__tml__check` / `mcp__tml__test` in Cortex pipelines.

## Debugging

### Compiler Diagnostics

```bash
tml check app.tml
tml lint src/
tml emit-ir app.tml --filter=main
```

### Profiling

```bash
tml test --profile
# Outputs Chrome DevTools JSON for Tracy or Chromium DevTools
```

### Debug Symbols

```bash
tml build app.tml    # Includes .pdb (Windows) or DWARF (Linux/Mac)
```

## Cleanup

```bash
# Remove build artifacts
tml clean

# Or manual:
rm -rf build/debug build/release
rm -rf .tml/        # Cache
```

Note: LLVM build (`build/llvm/`) is NOT cleaned by default (one-time cost).
