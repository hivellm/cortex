# Tml — Data & Storage

## File Formats

### `.tml` — Source Files

TML source code files with LL(1) grammar for deterministic parsing. No ambiguity = better LLM code generation.

```tml
use std::json::Json

pub func main() {
    let data = Json::parse("{\"name\": \"Tml\"}")
    println("Compiled by Tml 1.0")
}
```

### `.rlib` — TML Library Format

Compiled library with embedded metadata:

- Type information (signatures, generics, visibility)
- Documentation (from `///` comments)
- Symbol table for linking
- Fingerprints for incremental builds
- Binary IR for downstream optimization

Structure: ZIP archive containing:
- `metadata.json` — package info, dependencies, version
- `lib.a` or `lib.o` — compiled object code
- `doc.json` — extracted documentation
- `ir.ll` — LLVM IR (optional, for analysis)

Created by: `tml build --crate-type=rlib`

### `tml.toml` — Project Manifest

TOML file defining project metadata:

```toml
[package]
name = "cortex-tml"
version = "1.0.0"
edition = "2025"
description = "TML support for Cortex"

[dependencies]
# Future: external package management

[dev-dependencies]

[[bin]]
name = "app"
path = "src/main.tml"
```

## Compilation State

### Build Directory Structure

```
./build/
├── debug/
│   ├── app.exe              # Executable
│   ├── app.pdb              # Debug symbols
│   └── .tml/                # Query cache
├── release/
│   ├── app.exe
│   └── .tml/
├── llvm/                    # Vendored LLVM (one-time build)
│   ├── lib/
│   ├── include/
│   └── bin/
└── CMakeFiles/
```

### Query Cache (Red-Green Incremental)

`.tml/` directory stores:

- **Fingerprints**: Source/AST/HIR/MIR hashes
- **Cached LLVM IR**: Object files for unchanged modules
- **Metadata**: Type info, documentation

Cache invalidation via `tml cache invalidate` or `tml clean`.

## Data Structures in Std Library

### Collections

```tml
use std::collections::{List, HashMap, HashSet, Queue, Stack, BTreeMap}

let arr = List[I32]::new()      // Dynamic array
let map = HashMap[Str, I32]::new() // Hash map
let set = HashSet[Str]::new()   // Hash set
```

### JSON (SIMD-Optimized)

```tml
use std::json::Json

let data = Json::parse("{\"key\": 42}")
let value = data.get_i64("key")  // 42
let json_str = data.to_string()  // Round-trips cleanly
```

### File I/O

```tml
use std::file::{File, Path}

let path = Path::new("data.json")
let content = File::read_to_string(path)!
```

### Encoding Formats

Standard library supports 14+ formats:
- **Text**: UTF-8, percent encoding, base32/36/58/62/85/91
- **Binary**: msgpack, protobuf, base64, hex
- **Compression**: gzip, brotli, zstd, deflate

## State Management

### Persistent Storage

TML has no built-in persistence layer; use:
- **SQLite** (`std::sqlite`) for structured data
- **File I/O** (`std::file`) for documents
- **JSON** (`std::json`) for configuration

### Memory Model

- **Stack**: Local variables, function parameters
- **Heap**: Allocated via `Heap[T]`, `Box[T]` equivalent
- **Shared**: Thread-safe via `Shared[T]` (Arc-like) or `Sync[T]`
- **Interior Mutability**: `Cell[T]`, `RefCell[T]` for non-owning mutations

No garbage collection; all memory managed at compile-time via borrow checker.
