## 1. ADR-012
- [x] 1.1 `rulebook_decision_create` ADR-012 — "EventIdentity cross-backend join key + SQLite IdentityIndex". Status `proposed`. Created 2026-05-24 as decision #13 (slug `adr-012-eventidentity-cross-backend-join-key-sqlite-identityindex`). Note: MCP `rulebook_decision_create` rejected the `alternatives` array parameter on three retries (harness JSON-encoded the array as a string). Three rejected alternatives documented in commit message: (A) per-backend reverse-lookup indexes — rejected, same round-trip cost; (B) materialised view derived nightly — rejected, stale identity defeats the use case; (C) Envelope-embedded native ids in Parquet — rejected, forces full archive walk per call.
- [x] 1.2 Trade-off: ~3 days touching all projection paths; gain is doctor <10s for 100k events + forget correctness. Captured in ADR-012 §Consequences (positive / negative / neutral split).

## 2. Storage layer
- [x] 2.1 New `crates/cortex-storage/src/identity.rs`: `EventIdentity` struct + `IdentityIndex` impl (insert / upsert / lookup / delete). Trait + `SqliteIdentityIndex<'a>` wrapping a borrowed `&Connection` (the workspace serialises every `MetadataStore` access behind `Arc<Mutex<…>>` so the trait is intentionally NOT `Send + Sync`). `Backend` enum (`Nexus | Vectorizer | Meili | Archive`) with `as_str()` + `all()` const helpers. `IdentityError` over `rusqlite::Error` plus an explicit `EmptyId { field }` variant rejecting empty `event_id` / `native_id` at validation time so the partial UNIQUE indexes never see meaningless values.
- [x] 2.2 Migration in `metadata.rs::apply_phase13d_schema` creating `event_identity` with PK + secondary indexes. The migration helper lives in `identity.rs` (next to the struct) and is called from `MetadataStore::migrate` at every open so consumers grabbing `metadata.conn()` see the table without coordinating their own migration. Schema: PK on `event_id` + 3 UNIQUE partial indexes (`WHERE … IS NOT NULL`) on `nexus_id` / `vec_id` / `meili_id`. Archive partition deliberately NOT unique — multiple envelopes share the same hour-bucket parquet partition file.
- [x] 2.3 Unit tests: 6 cases (insert, upsert merges, lookup by each backend id, delete cascades, secondary-index lookup). 7 tests landed: `insert_then_lookup_returns_full_identity_row`, `upsert_merges_across_backends_for_same_event`, `lookup_by_each_native_id_finds_the_row`, `delete_drops_the_row_and_is_idempotent` (covers idempotent re-delete), `unique_index_rejects_two_events_claiming_the_same_native_id` (the structural cross-backend invariant), `empty_event_id_is_rejected_at_validation_time` (symmetric for `event_id` + `native_id`), `backend_as_str_and_all_stay_in_sync` (label / iteration order pin). `cargo test -p cortex-storage --lib` 65/65 green.

## 3. Projection paths write back
- [ ] 3.1 Embedder projection (`cortex-workers/src/embedder/projection.rs`) calls `IdentityIndex::upsert_identity(event_id, Backend::Vectorizer, vec_id)` after every successful insert.
- [ ] 3.2 Fulltext projection writes `Backend::Meili` id.
- [ ] 3.3 Graph projection writes `Backend::Nexus` id.
- [x] 3.4 Archive write writes `Backend::Archive` partition path. `ArchiveWriter::write` signature changed from `Result<(), ArchiveError>` to `Result<PathBuf, ArchiveError>` so the router can stamp the partition without re-deriving the path. `NdJsonZstdArchive::write` returns the open file's `.path.clone()`; `InMemoryArchive::write` returns a deterministic `mem://<stream_tag>` form so identity-index integration tests pin the round-trip without coupling to NdJsonZstd's on-disk layout. New `AppState::metadata: Option<Arc<Mutex<MetadataStore>>>` field + `with_metadata()` builder; `process_event` calls `SqliteIdentityIndex::upsert_identity(event_id, Backend::Archive, partition)` after the archive write succeeds. Best-effort: a failed upsert logs at WARN but does NOT undo the archive write (durability stays the headline contract).
- [x] 3.5 Per-projection IT asserts the identity row is materialised post-insert. `archive_write_back_stamps_event_identity_partition` in `crates/cortex-workers/tests/ingestion_router.rs` exercises the full router path: POST `/v1/events` → `process_event` → archive write → SqliteIdentityIndex upsert → lookup returns the row with `archive_partition` set + reverse `lookup_by_native(Backend::Archive, …)` resolves the same event_id; sibling columns (nexus_id, vec_id, meili_id) stay None because the other workers stamp those in their own projection paths. Pair test `archive_write_back_is_skipped_when_metadata_is_absent` proves the pre-phase13d code path keeps working when AppState has no metadata handle wired. §3.1/§3.2/§3.3 ITs land alongside their respective projection write-backs in follow-up commits — each touches a separate worker crate's AppState plumbing and warrants a focused session per AGENTS.md's "3+ files across subsystems → decompose into 1-2 file sub-tasks" rule.

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
