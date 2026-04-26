# cortex-storage

> Spec: [`docs/specs/02-storage-layout.md`](../../docs/specs/02-storage-layout.md)

The single source of truth for **where** Cortex data lives. Every backend
namespace (Vectorizer collection, Nexus label, Meilisearch index, Synap
stream/topic/KV namespace, Parquet partition, SQLite metadata table, CAS
blob store) is declared here and consumed by the workers and ops CLI.

If a worker hardcodes its own collection name, stream name, or schema, it
is wrong. It must use `cortex-storage`.

## Modules

| Module        | Owns                                                                                  |
|---------------|---------------------------------------------------------------------------------------|
| `collections` | Vectorizer collection names, dimensions, metric, per-collection schema (tier filters).|
| `graph`       | Nexus labels + bootstrap Cypher (constraints, indexes).                               |
| `fulltext`    | Meilisearch index names + the canonical `settings.v1.json`.                           |
| `streams`     | Synap streams, pub/sub topics, and KV namespaces.                                     |
| `archive`     | Parquet archive partition layout and rotation rules (NDJSON+zstd today, Parquet v2).  |
| `metadata`    | SQLite schema for the metadata store (sessions, events, runs).                        |
| `cas`         | SQLite-backed content-addressable blob store with zstd compression.                   |
| `names`       | Re-exports of well-known constant names so callers do not stringly-type them.         |

## Usage

```toml
[dependencies]
cortex-storage = { path = "../cortex-storage" }
```

```rust
use cortex_storage::{COLLECTIONS, MetadataStore, CasStore};

// Iterate Vectorizer collections to bootstrap.
for c in COLLECTIONS {
    println!("{}: dim={} metric={:?}", c.name, c.dim, c.metric);
}

// Open the metadata DB.
let meta = MetadataStore::open("./data/metadata.sqlite")?;

// Open the CAS for raw blob persistence.
let cas = CasStore::open("./data/cas.sqlite")?;
```

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Bootstrap workflow

1. The operator runs `cortex-ops plan --slice all` (see `cortex-ops`) to
   serialize the layout this crate declares.
2. `bin/cortex-init.sh` consumes that JSON and creates the corresponding
   resources in each backend (Vectorizer create-collection, Nexus Cypher,
   Meilisearch settings, Synap streams).
3. Workers then start and resolve their targets via the constants exposed
   by this crate, never by environment variables alone.

This makes the layout reviewable in code, reproducible across environments,
and impossible to silently drift.

## Schemas

JSON files under [`schemas/`](schemas/) ship the canonical Vectorizer
collection schemas and Meilisearch settings. They are loaded at compile
time and exposed through the typed APIs in this crate.

## Testing

```bash
cargo test -p cortex-storage
```

Round-trip tests guarantee that every JSON schema bundled in the crate
parses into the typed representation and that the bootstrap Cypher is
syntactically valid.

## Stability

Pre-1.0 — adding a new collection / index / stream is a minor change;
removing or renaming one is a breaking change for every worker.
