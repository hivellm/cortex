# Proposal: phase2_per_project_collection_isolation

## Why

Today every repo bootstrapped through `cortex-bootstrap` writes into the same six Vectorizer collections (`cortex-code`, `cortex-docs`, `cortex-decisions`, `cortex-governance`, `cortex-misc`, `cortex-turns`) and the same Meilisearch indexes. Confirmed against the live stack on 2026-04-27 just before wipe:

```
cortex-docs       → 69 817 vectors  (Cortex spec, Tml LLVM tests, CompressionPrompt benchmarks, …)
cortex-code       → 33 689 vectors
cortex-turns      → 17 319 vectors
total             → 120 877 vectors / ~282 MB
```

The walker classifies by extension only (`crates/cortex-bootstrap/src/walker.rs:293`), so 17 unrelated repos pile their `.md`/`.txt`/`.rst` content into `cortex-docs`. Searching for "embedder routing" inside the Cortex repo competes with LLVM test text from Tml in the same lane. There is no way to scope a query to "this repo only" because the lane fundamentally has no per-repo dimension.

Net effect: results are noisy, embed cost is multiplied by an order of magnitude, and the institutional memory promise — "this repo's decisions, this repo's similar turns" — breaks the moment a second repo is captured.

## What Changes

Per-project collection isolation. Every Vectorizer collection and Meilisearch index becomes `cortex-{repo_slug}-{family}`, where `repo_slug` is the canonical lowercased+slugged repo id from `cortex.toml.cortex.id` (or the git-root basename when absent).

Examples:

```
cortex-docs                 → cortex-cortex-docs, cortex-tml-docs, cortex-vectorizer-docs, …
cortex-code                 → cortex-cortex-code, cortex-tml-code, …
cortex-turns                → cortex-cortex-turns, cortex-tml-turns, …
```

Implementation surface:

- `cortex-storage::names`: add `slug_for_repo(repo_id: &str) -> String` (lowercase ASCII + non-`[a-z0-9-]` collapsed to `-`).
- `cortex-embedder::routing`: `collection_for(kind, prefix, repo_slug)` and `collection_for_chunk(kind, source, prefix, repo_slug)` gain the `repo_slug` parameter; format becomes `"{prefix}-{repo_slug}-{family}"`.
- `cortex-embedder::embedder`: chunkers thread `event.context_repo` through; events without `context_repo` route to an `"unknown"` slug and emit a warn.
- `cortex-fulltext::routing`: `index_for(prefix, kind)` becomes `index_for(prefix, kind, repo_slug)`.
- `cortex-fulltext::meili_client::ensure_index`: per-event lazy ensure replaces the upfront bootstrap of a fixed family list — collections appear on first write.
- `cortex-embedder::vectorizer_client::ensure_collection`: same lazy pattern.
- `cortex-api::strategies` (read side): builds lane requests using `req.scope.repos[0]` to pick the per-repo collection / index. Empty scope falls back to an `"unknown"` slug — empty results for unscoped queries until the orchestrator's multi-repo fan-out lands.

Nexus does not need collection-per-repo: nodes already carry a `repo` property; per-repo isolation at read time is a query-side filter (tracked under `phase2_scope_repo_resolution`).

## Impact

- Affected specs: spec-03 (local stack collections), spec-06 (embedder), spec-08 (fulltext indexer), spec-09 (bootstrap), spec-11 (lane wiring).
- Affected code:
  - `crates/cortex-storage/src/names.rs` (new helper)
  - `crates/cortex-embedder/src/routing.rs` (signature change)
  - `crates/cortex-embedder/src/{chunker_code,chunker_doc,chunker_fallback,embedder}.rs`
  - `crates/cortex-fulltext/src/routing.rs` (signature change)
  - `crates/cortex-fulltext/src/meili_client.rs` (lazy ensure)
  - `crates/cortex-api/src/strategies.rs` (read-side per-repo lane requests)
  - tests in each crate
- Breaking change: YES (collection / index names change). Existing data was wiped on 2026-04-27 prior to landing this — the fresh bootstrap repopulates per-repo from scratch.
- User benefit: bootstrap a single repo and its content lives only in `cortex-{slug}-{family}` collections. Query results no longer mix unrelated projects. Rebootstrapping a new repo only touches its own collections, nothing else.

## Source

2026-04-27 audit; 6 collections / 120 877 vectors covering 17 repos under a shared lane confirmed via `GET /collections` against the live Vectorizer.
