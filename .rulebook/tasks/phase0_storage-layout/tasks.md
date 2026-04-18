## 1. Constants + namespaces
- [ ] 1.1 Define `cortex-core::storage::names` with every collection / index / stream / KV namespace
- [ ] 1.2 Prefix convention (`cortex-` default, overridable per deployment)
- [ ] 1.3 `schema_version` constant propagated into keys where cache-invalidation requires it

## 2. Parquet archive writer
- [ ] 2.1 Partition layout `cortex.events/<yyyy>/<mm>/<dd>/<kind>.parquet`
- [ ] 2.2 Parquet writer wrapper with row-group size + compression defaults
- [ ] 2.3 Atomic rotate on day boundary

## 3. Synap stream topology
- [ ] 3.1 Declarative topology YAML (streams, retention, partitions)
- [ ] 3.2 Helper to `ensure_streams()` at startup (idempotent)

## 4. Vectorizer / Nexus / Meilisearch schemas
- [ ] 4.1 Vectorizer: declarative `CollectionSchema` per collection
- [ ] 4.2 Nexus: constraint + index bootstrap statements (see spec 07)
- [ ] 4.3 Meilisearch: `settings.v1.json` per index (see spec 08)

## 5. SQLite metadata store
- [ ] 5.1 Migration for `classifier_spend`, `trust_scores`, `materialized_view_cache`
- [ ] 5.2 `.cortex/meta.sqlite` location + backup policy
- [ ] 5.3 Retention policy table (by `pii_risk`: low → indefinite, medium → 365d, high → 30d raw)

## 6. Tail (mandatory)
- [ ] 6.1 Update `docs/specs/02-storage-layout.md` status flag to 🟢 + index row
- [ ] 6.2 Integration test: `ensure_*()` helpers are idempotent on a fresh + warm stack
- [ ] 6.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; ≥95% coverage on storage module
