# cortex-workers

> Specs: [`05-classifier.md`](../../docs/specs/05-classifier.md),
> [`06-embedder.md`](../../docs/specs/06-embedder.md),
> [`07-graph-writer.md`](../../docs/specs/07-graph-writer.md),
> [`08-fulltext-indexer.md`](../../docs/specs/08-fulltext-indexer.md)

The four daemons that drive the Cortex event pipeline, consolidated into
one crate. Each one consumes from `cortex.events.*` (Synap) and writes
into a backend that `cortex-storage` declares.

If a `docker-compose` or systemd unit used to call
`cortex-classifier-worker`, the binary name is unchanged; only the source
crate moved. Library imports are
`cortex_workers::{classifier, embedder, fulltext, graph}`.

## Workers

| Binary                       | Module       | Reads                                | Writes                                                   |
|------------------------------|--------------|--------------------------------------|----------------------------------------------------------|
| `cortex-classifier-worker`   | `classifier` | `cortex.events.raw`, `…bootstrap`    | `cortex.events.enriched` (after `cortex-classifier`).    |
| `cortex-embedder-worker`     | `embedder`   | `cortex.events.enriched`             | Vectorizer collections (chunked + embedded).             |
| `cortex-fulltext-worker`     | `fulltext`   | `cortex.events.enriched`             | Meilisearch indexes.                                     |
| `cortex-graph-worker`        | `graph`      | `cortex.events.enriched`             | Nexus nodes + edges.                                     |
| `cortex-graph-backfill`      | `graph`      | Snapshot scan                        | Nexus, for one-shot reindex of historical events.        |

`EnrichedEvent` is canonically defined in `embedder` and re-exported by
`fulltext` and `graph` so all three consumers see the same shape.

## Modules

- **`classifier`** — bridge that takes raw + bootstrap events, runs them
  through the `cortex-classifier` stack (Haiku CLI by default, static
  fallback), and republishes as `enriched`.
- **`embedder`** — chunker + embedder. Uses `tree-sitter` for Rust /
  TypeScript / JavaScript / Python / Go / Java / C / C++ / Markdown /
  JSON / YAML / TOML and calls the Vectorizer SDK to upsert vectors.
- **`fulltext`** — projects enriched events onto Meilisearch documents
  following the index settings declared in `cortex-storage`.
- **`graph`** — projects enriched events onto Nexus nodes and edges; the
  backfill bin replays history into Nexus when the schema changes.
- **`admin_health`** — shared admin/health HTTP server (uses
  `cortex-health` with the `server` feature) so each worker exposes the
  same `/healthz` and `/admin` surface.

## Usage

```bash
# Run individual workers
cargo run -p cortex-workers --bin cortex-classifier-worker
cargo run -p cortex-workers --bin cortex-embedder-worker
cargo run -p cortex-workers --bin cortex-fulltext-worker
cargo run -p cortex-workers --bin cortex-graph-worker

# One-shot graph backfill
cargo run -p cortex-workers --bin cortex-graph-backfill -- --since 2026-01-01
```

As a library:

```rust
use cortex_workers::embedder::{Worker, WorkerConfig};

let cfg = WorkerConfig::from_env()?;
Worker::new(cfg).run().await?;
```

The crate is `#![forbid(unsafe_code)]`.

## Configuration

Each worker reads the standard backend URLs (`SYNAP_URL`,
`VECTORIZER_URL`, `NEXUS_URL`, `MEILISEARCH_URL`) plus its own knobs
documented in the matching spec. Storage targets — collection names,
index names, stream names — are **never** environment-driven; they come
from `cortex-storage` constants.

Cypher seeds for Nexus live in [`cypher/`](cypher/); Meilisearch settings
live in [`settings/`](settings/). Both are loaded through `cortex-storage`
so workers and the `cortex-ops doctor` see the same source of truth.

## Testing

```bash
cargo test -p cortex-workers
```

Integration tests use `wiremock` to fake the HTTP surfaces of
Vectorizer / Nexus / Meilisearch so the suite stays hermetic. Tree-sitter
chunking, classifier bridging, and graph projection have unit coverage
per module.

## Stability

Pre-1.0 — adding a new worker is a minor change; changing the projection
from `EnrichedEvent` onto a backend is a breaking change for that
backend's consumers and usually requires a `cortex-graph-backfill`-style
replay.
