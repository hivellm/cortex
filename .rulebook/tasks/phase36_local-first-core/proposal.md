# Proposal: phase36_local-first-core

## Why
The daily-only runtime disconnected months of collected history while the full system requires twelve Docker services. The operator explicitly requests replacing that architecture with one self-hosted CLI, embedded web GUI, local files and indexes, Jev classification and simple graph links, planned with Claude and implemented.

## What Changes
Build a dependency-light standalone Cortex engine with canonical local Markdown records, rebuildable SQLite FTS5 and typed links; resumably import existing daily SQLite and compressed NDJSON history without changing sources; integrate bounded Jev classification; expose direct CLI, MCP, lifecycle capture and embedded local web UI through one executable; switch default packaging and plugins to the standalone core and verify real historical recall with Docker stopped.

## Impact
- Affected specs: Local canonical store, rebuildable retrieval, provenance-preserving history migration, single-process runtime and Jev classification contracts.
- Affected code: New standalone Rust core and executable; default workspace/build/install/plugin integration; local import/index/classification/web/MCP modules; existing legacy runtime retained solely for compatibility and source recovery.
- Breaking change: Default runtime and installation change from multiple executables/services to standalone Cortex; preserve old source data and document compatibility.
- User benefit: Historical data and daily agent memory queryable without Docker or separate data services, with transparent files and a built-in web interface.
