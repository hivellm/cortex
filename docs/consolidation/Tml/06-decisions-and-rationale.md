# Tml — Design Decisions & Rationale

## LL(1) Grammar for LLM Code Generation

**Decision**: Grammar is strictly LL(1) — deterministic with one-token lookahead, no backtracking.

**Rationale**: Language models generate code token-by-token. Backtracking parsers (LR, LALR) confuse LLMs because they require lookahead and reduction decisions that aren't obvious from local context. LL(1) means:
- LLMs can generate TML code with fewer syntax errors
- Parser and model agree on "next valid token" at every position
- No ambiguous constructs like C's dangling `else` or Rust's `>>`

**Examples of ambiguity removed**:
- `[` always starts a generic type, never array indexing in declaration
- `when` keyword prevents confusion with `switch` (Go vs Rust idiom)
- `do` for closures avoids `|...|` (can be bitwise OR)

## Native MCP Server in Compiler

**Decision**: Implement full MCP 2.0 server (JSON-RPC, stdio transport) directly in the compiler binary.

**Rationale**:
- No separate language server or wrapper process
- Same code paths as CLI: `mcp__tml__check` runs the real type checker
- Direct access to compiler internals (query cache, HIR, MIR, LLVM IR)
- Enables deterministic AI-assisted code generation at scale

## THIR (Typed High-Level IR) as First-Class Pipeline Stage

**Decision**: Insert THIR between HIR and MIR as an explicit query stage.

**Rationale**:
- **Materializes coercions**: `I8 + I32` becomes explicit `CoercionExpr`
- **Resolves dispatch**: `x.to_string()` resolved to `Display::to_string(x)`
- **Exhaustiveness checking**: Maranget 2007 algorithm catches missing patterns early
- **Foundation for optimization**: MIR generation sees explicit, normalized code

Inspired by rustc but integrated as a permanent architecture, not optional.

## Red-Green Incremental Compilation

**Decision**: Use fingerprint-based demand-driven query system (like rustc's salsa).

**Rationale**: 22ms cached builds for large projects:
- Each compilation stage is a memoized query
- Input fingerprints determine cache hit/miss
- Change propagates only if downstream fingerprints differ
- Critical for AI-assisted workflows where user edits code → re-run → check → repeat

## One Binary, Zero External Tools

**Decision**: Embed LLVM, linker, formatter, linter, test runner, profiler, MCP server in single executable.

**Rationale**:
- Eliminates tool versioning mismatches
- Installation is one download
- Ideal for containerized Cortex pipelines (Docker image has everything)
- Performance: startup time is zero (no subprocess spawning)

## Subprocess Test Architecture

**Decision**: Compile test suites to standalone EXEs, run as subprocesses, stream results via NDJSON.

**Rationale**:
- Crash isolation: one failing test doesn't kill others
- Parallel execution across cores (implied by subprocess model)
- Language-agnostic protocol (JSON) for result parsing
- Hash-based caching skips unchanged suites (8s full suite with cache)

## No Null Pointers

**Decision**: Use `Maybe[T]` (like Option in Rust) instead of nullable references.

**Rationale**:
- Eliminates null pointer dereference crashes (a category of bugs)
- Forces explicit handling of absence
- Makes code intent clear (Maybe = could be Nothing, not null)
- Aligns with Haskell, Rust, OCaml — languages LLMs understand well

## Move Semantics + Borrow Checker

**Decision**: Adopt Rust's ownership + borrowing model.

**Rationale**:
- Memory safety at compile-time, no GC overhead
- Deterministic performance (no pause times)
- Explicit about who owns what resource
- Prevents data races at language level

## Self-Documenting Syntax

**Decision**: Keywords and operators chosen to minimize LLM confusion.

Examples:
- `and` instead of `&&` (natural language, avoids bitwise confusion)
- `when` instead of `match` (Rust pattern, clearer intent)
- `func` instead of `fn` (self-documenting)
- `behavior` instead of `trait` (describes what it is)
- `Heap[T]` instead of `Box<T>` (describes storage location)

**Rationale**: LLMs are trained on natural language. Keyword choices that align with English reduce syntax hallucinations.

## Integrated Documentation Search

**Decision**: Hybrid BM25 + HNSW vector search for 6000+ API items, sub-10ms query latency.

**Rationale**:
- AI-assisted code generation needs fast, semantic API discovery
- BM25 (lexical) handles exact queries; HNSW (semantic) handles paraphrase queries
- Cached to disk; no network latency
- Built into compiler, not external service

Example: searching `"send data async"` finds `AsyncBufWriter::write()` and `Channel::send()` even without exact keyword match.

## Zig as Default Build Toolchain

**Decision**: Default to Zig 0.15.2 (Clang 20 + bundled libc) for C/C++ compilation.

**Rationale**:
- Zig CC (Clang) is fastest and most portable
- Bundles its own libc (no system dependency hell)
- Falls back to MSVC or system Clang if unavailable
- Stable version pinning (0.15.2, NOT 0.16+) prevents runtime linker issues

## HTTP Server as Standard Library Component

**Decision**: Ship full HTTP/1.1 + WebSocket + HTTP/2 server (not just client) in stdlib.

**Rationale**:
- Enables TML as first-class web language (183K req/s)
- AI-assisted API design and microservice development
- Radix tree router + middleware pipeline for app development
- Useful for Cortex worker services written in TML
