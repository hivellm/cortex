# Tml — Overview

## Purpose

**TML (To Machine Language)** is a batteries-included, AI-optimized programming language and compiler built for the AI era. It is designed as a first-class integration point between language models (LLMs) and software development, shipping a native MCP (Model Context Protocol) server directly inside the compiler.

## Role in HiveLLM

Tml serves as the language runtime for code generation and execution within HiveLLM projects. It is not a service that Cortex calls; rather, it is a **compilation and execution target** for code that needs to run deterministically and at scale.

## Stack

- **Compiler**: C++20 (240,000+ lines), in-process LLVM backend (55+ static libs)
- **Runtime**: C with Go-inspired concurrency (channels, atomics, async/await)
- **Standard Library**: 150,000+ lines of TML (core + std modules)
- **Test Framework**: Subprocess-based, NDJSON protocol, 12,000+ tests
- **Profiler**: Tracy integration (70+ instrumented zones)
- **Build**: Zig 0.15.2 (default), MSVC 19.30+, or Clang 15+

## Maturity

**Status**: C++ compiler is **100% functional (beta)**. All language features, standard library, and tooling are fully implemented and test-covered.

Self-hosted TML compiler (written in TML itself) is actively in development on `feat/self-hosting-compiler` branch. Core compiler performance: 22ms cached builds, 4.5x faster than `cargo check`.

## Key Characteristics

- **One binary**: No external tools needed (compiler, linker, formatter, linter, tests, profiler, MCP server all built-in)
- **Deterministic**: LL(1) grammar eliminates ambiguity for LLM code generation
- **Performance**: HTTP server achieves 183K req/s, faster than Node.js
- **Memory safety**: Compile-time borrow checking + type safety, no null pointers
- **AI-first**: Native MCP server (14 tools), semantic documentation search (BM25 + HNSW)
