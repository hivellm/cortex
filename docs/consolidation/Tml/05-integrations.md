# Tml — Integrations

## Cortex Walker Integration

**Recent commit `7fd3c62`** added support for indexing `.tml` files in Cortex Walker:

```
feat(walker,docker): index .tml files + fix Dockerfile claude-archive feature
```

### What This Means

Cortex Walker now crawls repositories and extracts `.tml` source files for Cortex index ingestion. This enables:

- **Full-text search** over TML codebases
- **Semantic search** combining Tml documentation (via `mcp__tml__docs/search`)
- **Code analysis** by running `mcp__tml__check` or `mcp__tml__emit-ir` on indexed files

### Expected Cortex Integration Flow

1. Walker discovers `.tml` files in bootstrap repos
2. Extracts source + metadata (file path, package, type signatures)
3. Indexes into Meili (lexical) + Vectorizer (semantic embeddings)
4. Cortex API surfaces results for LLM-assisted TML code generation

## MCP Tools

Tml exports 14 MCP tools via native compiler integration (not wrappers):

| Tool | Purpose | Cortex Use |
|------|---------|------------|
| `mcp__tml__check` | Fast type-check | Validate TML code before indexing |
| `mcp__tml__test` | Run test suite | Verify indexed examples work |
| `mcp__tml__emit-ir` | LLVM IR generation | Analyze code structure |
| `mcp__tml__docs/search` | Semantic doc search | Find APIs by meaning (BM25 + HNSW) |
| `mcp__tml__compile` | Full compilation | Build executables from indexed source |
| `mcp__tml__format` | Code formatting | Normalize indexed code |
| `mcp__tml__lint` | Semantic linting | Flag style/complexity issues |

## External Extensions

### TmlDocs

Auto-generated documentation system:

```bash
tml doc src/ --format=html
tml doc src/ --format=json
tml doc src/ --format=markdown
```

Outputs:
- HTML: Interactive browsable docs (like Rust docs.rs)
- JSON: Machine-readable API schema
- Markdown: For GitHub wikis

Used for semantic search indexing (`docs/search` MCP tool).

### TmlTextmate (VSCode Syntax)

Language server for VSCode integration:

- Syntax highlighting for `.tml` files
- IntelliSense (type hints, completions)
- Diagnostics (errors, warnings, hints)
- Code navigation (goto definition, find references)

Repository: `vscode-tml/` in Tml project.

## Hive SDK Interoperability

TML has FFI bindings to C/C++; potential integration points:

- **Vectorizer SDK**: Call Vectorizer embedding APIs from TML
- **Nexus SDK**: Query Nexus graph from TML (external IDs support)
- **Synap SDK**: Read/write to Synap knowledge graphs
- **Expert SDK**: Call Expert inference engines

Example (not implemented yet):

```tml
use std::ffi::{CStr, c_void}

// FFI to Vectorizer C API
pub extern "C" func embed(text: CStr) -> Slice[F32]
```

## Cortex Walker Configuration

Walker must be configured to discover `.tml` files:

```toml
# hypothetical walker config
[[sources]]
path = "e:\\HiveLLM\\Tml\\lib"
extensions = [".tml"]
index_type = "source"

[[processors]]
type = "tml"
run_check = true
extract_docs = true
```

Ensures `.tml` source files are indexed with type information and documentation.

## Documentation Indexing

Tml's native MCP `docs/search` tool enables:

- **Sub-10ms queries** across 6000+ API items
- **Hybrid ranking**: BM25 (lexical) + HNSW (semantic)
- **Query expansion**: 65+ TML-specific synonyms
- **MMR diversification**: Avoid redundant results

This becomes Cortex's source of truth for TML stdlib API discovery.
