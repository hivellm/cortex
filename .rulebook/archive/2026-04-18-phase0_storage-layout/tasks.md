## 1. Constants + namespaces
- [x] 1.1 Define `cortex-storage::names` with every collection / index / stream / KV namespace
- [x] 1.2 Prefix convention (`cortex-` default, overridable per deployment)
- [x] 1.3 `schema_version` constant propagated into the metadata store

## 2. Parquet archive layout
- [x] 2.1 Partition layout `events/year=<yyyy>/month=<mm>/day=<dd>/hour=<hh>/`
- [x] 2.2 Partition + filename helpers in `archive.rs`
- [x] 2.3 Compression codec + level constants documented; actual writer lives in cortex-core (phase0_cortex-core)

## 3. Synap stream topology
- [x] 3.1 Declarative `StreamConfig` array with retention + partitions
- [x] 3.2 Declarative `KvNamespace` array with TTLs

## 4. Vectorizer / Nexus / Meilisearch schemas
- [x] 4.1 Vectorizer: `CollectionSchema` per collection in `collections.rs`
- [x] 4.2 Nexus: label + relationship vocabulary + bootstrap Cypher in `graph.rs`
- [x] 4.3 Meilisearch: eight `settings.v1.json` files embedded via `include_str!`

## 5. SQLite metadata + CAS store
- [x] 5.1 Embedded `schemas/sqlite/schema.sql` applied by `MetadataStore::open`
- [x] 5.2 `.cortex/meta.sqlite` layout conventionalized via `MetadataStore`
- [x] 5.3 Retention / spend / trust / laws / bootstrap tables created; `classifier_spend` + `sessions` helpers exercised
- [x] 5.4 SQLite-backed `CasStore` with Zstd-compressed blobs + refcount + vacuum

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation (spec 02 flipped to 🟢 in [docs/specs/00-index.md](../../../docs/specs/00-index.md) and [02-storage-layout.md](../../../docs/specs/02-storage-layout.md))
- [x] 6.2 Write tests covering the new behavior (31 unit tests across names / collections / graph / fulltext / streams / archive / metadata / cas)
- [x] 6.3 Run tests and confirm they pass (`cargo check && cargo clippy -p cortex-storage --all-targets -- -D warnings && cargo test -p cortex-storage` — 31 passing, 0 warnings)
