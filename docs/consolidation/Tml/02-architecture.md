# Tml — Architecture

## Compiler Pipeline

```
Source (.tml)
  ├─ Lexer (tokenize)
  ├─ Parser (LL(1), no backtrack)
  ├─ Type Checker (Hindley-Milner inference)
  ├─ Borrow Checker (NLL + Polonius)
  ├─ HIR (High-level IR — explicit ownership)
  ├─ THIR (Typed HIR — materializes coercions, resolves dispatch, exhaustiveness)
  ├─ MIR (Mid-level IR — SSA, 30+ optimization passes)
  └─ LLVM IR → Object Code → Link → Executable
```

## Incremental Compilation

Uses **demand-driven query system** with red-green fingerprinting (inspired by rustc):

- **QueryContext**: Each phase is a memoized function with dependency tracking
- **Fingerprinting**: Source changes tracked via hash; unchanged modules stay GREEN
- **RED phase**: Recompute changed inputs; only propagate if downstream fingerprints differ
- **Result**: 22ms cached builds for large projects

## MCP Server (Native in Compiler)

Built directly into the compiler as a JSON-RPC 2.0 server (stdio transport):

| Tool | Purpose |
|------|---------|
| `compile` / `build` / `run` / `check` | Compilation stages |
| `emit-ir` / `emit-mir` | Intermediate representation |
| `test` | Test execution with coverage/profiling |
| `format` / `lint` | Code style and semantic analysis |
| `docs/search` | Hybrid BM25 + HNSW semantic search |
| `docs/get` / `docs/list` / `docs/resolve` | Documentation retrieval |
| `cache/invalidate` | Compilation cache management |

Documentation search: 6000+ items, sub-10ms latency, query expansion (65+ synonyms), MMR diversification.

## Standard Library Structure

| Layer | Modules | Purpose |
|-------|---------|---------|
| **Core** | array, iter, option, result, str, slice, fmt, cmp, hash, cell, mem, alloc, encoding, future, simd | Foundational types and operations |
| **Std** | collections, file, http, json, net, hash, crypto, zlib, sqlite, regex, sync, thread, stream, aio, math, datetime, random, glob, os, log, search, msgpack, protobuf, promise, observable | Application-level services |

## Testing Architecture

Subprocess-based with NDJSON protocol:

1. Test suites compile to standalone EXEs
2. Each test runs as subprocess (crash isolation)
3. Results stream as structured JSON events
4. Hash-based caching skips unchanged suites
5. Coverage via `TML_COVERAGE_FILE` env var (no LLVM overhead)
6. Performance: 12,000+ tests in 8 seconds (cached)

## Memory Model

- **Ownership**: Move semantics (like Rust)
- **Borrowing**: Compile-time borrow checker (NLL + Polonius)
- **Smart pointers**: `Heap[T]`, `Shared[T]`, `Sync[T]`, `Cell[T]`, `RefCell[T]`
- **No null**: Use `Maybe[T]` instead of `null`
