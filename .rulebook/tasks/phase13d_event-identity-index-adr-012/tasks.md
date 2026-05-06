## 1. ADR-012
- [ ] 1.1 `rulebook_decision_create` ADR-012 — "EventIdentity cross-backend join key + SQLite IdentityIndex". Status `proposed`.
- [ ] 1.2 Trade-off: ~3 days touching all projection paths; gain is doctor <10s for 100k events + forget correctness.

## 2. Storage layer
- [ ] 2.1 New `crates/cortex-storage/src/identity.rs`: `EventIdentity` struct + `IdentityIndex` impl (insert / upsert / lookup / delete).
- [ ] 2.2 Migration in `metadata.rs::apply_phase13d_schema` creating `event_identity` with PK + secondary indexes.
- [ ] 2.3 Unit tests: 6 cases (insert, upsert merges, lookup by each backend id, delete cascades, secondary-index lookup).

## 3. Projection paths write back
- [ ] 3.1 Embedder projection (`cortex-workers/src/embedder/projection.rs`) calls `IdentityIndex::upsert_identity(event_id, Backend::Vectorizer, vec_id)` after every successful insert.
- [ ] 3.2 Fulltext projection writes `Backend::Meili` id.
- [ ] 3.3 Graph projection writes `Backend::Nexus` id.
- [ ] 3.4 Archive write writes `Backend::Archive` partition path.
- [ ] 3.5 Per-projection IT asserts the identity row is materialised post-insert.

## 4. Doctor + forget rewire
- [ ] 4.1 Rewrite `cortex doctor consistency` to walk `event_identity` once and call `exists(backend, id)` per backend.
- [ ] 4.2 Bench: 100k events finishes in <10s on the running stack. Test budget gate in CI.
- [ ] 4.3 `admin forget` reads `event_identity` and dispatches deletes per backend in one transaction.
- [ ] 4.4 IT: forget(event_id) → identity row absent + per-backend lookups all return Not-Found.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/04-event-schema.md` + new `docs/specs/25-event-identity.md` + `CHANGELOG.md`.
- [ ] 5.2 Tests: §2.3 + §3.5 × 4 + §4.4 + §4.2 budget guard.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
