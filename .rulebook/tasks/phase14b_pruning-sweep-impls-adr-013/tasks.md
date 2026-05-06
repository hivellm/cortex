## 1. ADR-013
- [ ] 1.1 `rulebook_decision_create` ADR-013 — "Vectorizer pruning is collection-level until SDK 3.2". Status `accepted`.
- [ ] 1.2 Trade-off documented: per-vector move blocked on Vectorizer 3.2; collection-level re-encode runs in O(collection_size) time but is correct.

## 2. Hot-tier prune as Sweep
- [ ] 2.1 Migrate `tier_sweep` to `impl Sweep for HotTierPrune`. Reads `event_identity` to dispatch per-backend deletes.
- [ ] 2.2 Default schedule `0 4 * * *`. Default cutoff: 90 days for hot tier.
- [ ] 2.3 Per-step IT against the in-memory backend.

## 3. Cold-tier prune as Sweep
- [ ] 3.1 New `impl Sweep for ColdTierPrune`. Default schedule `0 5 * * 0` (weekly Sunday 05:00 UTC). Default cutoff: 365 days.
- [ ] 3.2 For each `event_identity` row with `occurred_at < cutoff`, dispatch DELETE to Synap, Nexus, Meili, archive. Vectorizer handled per §4.
- [ ] 3.3 Post-cascade assertion: row absent in `event_identity` AND every backend reports Not-Found for the event_id.
- [ ] 3.4 IT covering 100 events: 30 hot + 70 cold; post-prune asserts cold ones are gone everywhere.

## 4. Vectorizer collection-level re-encode
- [ ] 4.1 New `crates/cortex-workers/src/embedder/vectorizer_prune.rs::reencode_collection(name, predicate)` that streams alive vectors out, drops the collection, rebuilds it from the stream.
- [ ] 4.2 Predicate is "occurred_at >= cutoff" expressed against the projected payload.
- [ ] 4.3 Atomicity: re-encoded collection is built under `<name>.tmp`, swapped on completion. Failure leaves the original intact.
- [ ] 4.4 Test against an in-memory Vectorizer fixture with 1k vectors (300 expired): post-prune count == 700 alive.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/02-quantization.md` + ADR-013 + `CHANGELOG.md`.
- [ ] 5.2 Tests: §2.3 + §3.4 + §4.4.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
- [ ] 5.4 Live smoke: backfill 100 expired events; run cold-tier prune; doctor consistency reports zero residue.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
