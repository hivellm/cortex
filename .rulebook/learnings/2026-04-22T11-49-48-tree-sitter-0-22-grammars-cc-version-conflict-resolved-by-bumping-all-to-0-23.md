# Tree-sitter 0.22 + grammars: cc version conflict resolved by bumping all to 0.23
**Source**: manual
**Date**: 2026-04-22
**Related Task**: phase1_embedder
**Tags**: build, tree-sitter, cargo-resolver, cortex-embedder, grammars
While scaffolding `cortex-embedder` in round 1, the initial pin of `tree-sitter = "0.22"` with grammar crates at `0.21` (rust/ts/js/python/go/java/c) plus `tree-sitter-toml-ng = "0.7"` broke cargo's resolver:

```
error: failed to select a version for `cc`.
  ... required by package `tree-sitter-toml-ng v0.7.0` (wants cc ^1.2)
  previously selected package `cc v1.0.90` required by
  `tree-sitter-javascript v0.21.0` (wants cc = ~1.0.90)
```

`tree-sitter-javascript 0.21` pins `cc = ~1.0.90` while `tree-sitter-toml-ng 0.7` requires `cc ^1.2` — these cannot coexist. The rest of the grammar ecosystem has moved to the `0.23` line.

**Resolution** (single retry, one-shot fix):
- `tree-sitter = "0.23"`
- `tree-sitter-rust = "0.23"`, `tree-sitter-typescript = "0.23"`, `tree-sitter-javascript = "0.23"`, `tree-sitter-python = "0.23"`, `tree-sitter-go = "0.23"`, `tree-sitter-java = "0.23"`, `tree-sitter-c = "0.23"`, `tree-sitter-cpp = "0.23"`
- `tree-sitter-json = "0.24"` (json moved one minor ahead of core)
- `tree-sitter-md = "0.3"` (markdown on the 0.3 line)
- `tree-sitter-yaml = "0.7"`, `tree-sitter-toml-ng = "0.7"`

**For future language additions**: check the top matching-minor `tree-sitter-$lang` version on crates.io before adding. If a grammar trails behind the core release, upgrading the core hits the same `cc` wall — prefer the grammar ecosystem's dominant major over the core crate's latest.

**File**: `crates/cortex-embedder/Cargo.toml`. Workspace check clean at `0.23`, 12 grammars loading lazily via `OnceLock<Language>` per language in `src/chunker_code.rs::CodeLanguage::language()`.