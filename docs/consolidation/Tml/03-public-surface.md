# Tml — Public Surface

## Language Syntax (Self-Documenting for LLMs)

### Core Keywords

| Concept | TML | Why |
|---------|-----|-----|
| Function | `func` | Self-documenting vs `fn` |
| Logical AND | `and` | Natural language vs `&&` |
| Logical OR | `or` | Natural language vs `\|\|` |
| Pattern match | `when` | Intent-revealing vs `match` |
| Loop | `loop` (unified) | One keyword, clear intent |
| Unsafe | `lowlevel {}` | Accurate descriptor |
| Range | `0 to 10` (exclusive), `0 through 10` (inclusive) | English-readable |
| Error propagate | `expr!` | Visible marker vs `expr?` |
| Pipe | `x \|> f` | Unix chaining |

### Types & Syntax

```tml
// Basic types: I8–I128, U8–U128, F32, F64, Bool, Char, Str
let name: Str = "TML"
let count: I32 = 42
let value: Maybe[I32] = Just(42)        // Option[T]
let result: Outcome[I32, Str] = Ok(100) // Result[T, E]

// Generics: [T] instead of <T> (no ambiguity)
func first[T](items: Slice[T]) -> Maybe[T] { ... }

// Pattern matching
when result {
    Ok(n) => println(n.to_string()),
    Err(e) => println("error: {e}")
}

// Closures with 'do'
numbers.map(do(x) x * 2)

// Behaviors (traits)
pub behavior Hashable {
    pub func hash(this) -> I64
}

// Template literals
let msg = `Hello, {name}! Count: {count}`
```

## MCP Tools (Cortex Walker Integration)

Available via `mcp__tml__*` in Cortex:

```bash
# Check-only (fast feedback)
tml check app.tml

# Full compilation
tml build app.tml
tml build app.tml --release

# Testing + coverage
tml test
tml test --coverage --profile
tml test --suite=core/str

# Formatting & linting
tml fmt src/
tml lint src/ --fix

# Documentation
tml doc lib/ --format=json

# Profiling
tml test --profile

# Daemon (22ms cached builds)
tml daemon start
tml daemon stop
```

## CLI Commands

### Build System

```bash
tml init [--bin|--lib] [--name NAME]
tml build [--release] [--target TRIPLE] [--crate-type lib|dylib|rlib]
tml run APP.tml
tml check APP.tml
tml clean
```

### Testing & Profiling

```bash
tml test [--coverage] [--profile] [--suite=core/str] [--filter=PATTERN]
tml bench [--baseline=NAME]
```

### Code Tools

```bash
tml fmt [--check] src/
tml lint [--fix] src/
tml doc src/ --format=html|json|markdown
```

### Daemon & Cache

```bash
tml daemon start
tml daemon stop
tml cache invalidate <FILES>
```

## Build Script (Windows)

**`scripts\build.bat`**: Single entry point for compilation

```bat
scripts\build.bat              # Debug
scripts\build.bat release      # Release
scripts\build.bat --clean      # Clean rebuild
scripts\build.bat --tests      # Build C++ unit tests
scripts\build.bat --zig        # Force Zig CC
scripts\build.bat --msvc       # Force MSVC
scripts\build.bat --clang      # Force Clang
```

Compiler detection order: Zig CC (fastest) > MSVC > Clang.

## Project File: `tml.toml`

TOML-based manifest for TML projects:

```toml
[package]
name = "myapp"
version = "1.0.0"
edition = "2025"

[dependencies]
# Tml standard library is built-in; no external dep management yet

[dev-dependencies]
```
