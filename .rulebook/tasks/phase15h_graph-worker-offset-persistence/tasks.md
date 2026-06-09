## 1. Persist the offset across recreates
- [ ] 1.1 Add a named volume for the worker metadata DB + set `CORTEX_GRAPH_METADATA_DB` to the mounted path in `docker-compose.yml` (cortex-graph-worker).
- [ ] 1.2 Verify the committed consumer offset survives a `docker compose up -d --no-deps cortex-graph-worker` recreate (no re-consume from 0).

## 2. Cold-boot at head, not zero
- [ ] 2.1 `LiveSynapConsumer::with_persistent_offset`: when `consumer_offset_lookup` returns `None`, seed the tracker at the current Synap stream head (latest), not 0 — history goes through `cortex-ops graph backfill`, never a full live re-projection.
- [ ] 2.2 If Synap 0.12 exposes a durable consumer-group cursor via the SDK, resume from it (`synap_group`) instead of the local metadata store; fall back to the metadata path otherwise.

## 3. Recovery runbook
- [ ] 3.1 Document how to seed the offset to head (`cortex-ops graph replay --since <head> --metadata-db <mounted>`) to restore the graph lane without re-processing history.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/07-graph-writer.md` (offset/resume) + `CHANGELOG.md`.
- [ ] 4.2 Tests: cold-boot-at-head seed + persisted-offset resume unit/IT.
- [ ] 4.3 `cargo check --workspace && cargo clippy -p cortex-workers -- -D warnings && cargo test -p cortex-workers` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
