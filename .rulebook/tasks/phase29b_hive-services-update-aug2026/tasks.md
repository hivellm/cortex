## 1. August-2026 service/SDK update round
- [x] 1.1 Synap SDK 1.0 → 1.3 (workspace pin `synap-sdk = "1.3"` + lockfile 1.3.0); workspace `cargo check --all-targets` clean — zero API breakage.
- [x] 1.2 Synap container 1.0.0 → 1.3.0 (compose pin + pull + recreate); `/health` reports 1.3.0, healthcheck green.
- [x] 1.3 Nexus decision recorded: STAY on 2.5.0/2.5.0 — Docker Hub carries only `3.0.0-alpha` (no stable 3.x) and crates.io has no `nexus-graph-sdk` 3.x; a major may migrate storage, and production data does not move onto an alpha without explicit user instruction. Marker for the future bump: when stable 3.0 + SDK publish, ALSO re-audit rmcp RUSTSEC-2026-0189 (hivellm/nexus#28) which is pinned by nexus-protocol.
- [x] 1.4 Post-restart incident handled: the synap-only recreate wiped its (ephemeral) stream rooms → ~2400 ERROR/min `Room not found` from the still-running consumers (their boot-time declare from 95f32c7 only runs at startup). Consumers restarted → re-declared → zero errors. Structural fix tracked as §2.
- [x] 1.5 All 7 cortex images rebuilt on the synap-sdk 1.3 workspace + redeployed; verified: 12/12 containers healthy, synap log ERROR-free, classifier `last_consume_ts_ms` age 0s (actively consuming).
- [x] 1.6 Spec 03 service-map row updated to `hivehub/synap:1.3.0`; CHANGELOG entry added under Changed.

## 2. Structural: survive synap restarts without room spam
- [x] 2.1 Runtime room re-declare shipped in all four live consumers (classifier/embedder/fulltext/graph `next_batch`): on `Room not found`, idempotent `get_or_create_room` + retry ONCE within the same poll (bounded per poll; a still-missing room degrades to the previous empty-batch idle, other errors surface unchanged). Publishers already had create-and-retry; consumers were the gap.
- [x] 2.2 `tests/synap_room_selfheal_it.rs` — fake Synap speaking the real SDK wire shape: (a) first consume Room-not-found → declare fires exactly once (wiremock expect(1)) → retried consume delivers the event, all inside one next_batch; (b) still-missing after declare → empty batch (idle), never an error, declare still bounded at one. 2/2 green.

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [x] 3.1 Docs: proposal carries the full decision record (incl. the deliberate Nexus non-bump and its future-bump marker); spec 03 pin row synced; CHANGELOG entry; self-heal contract documented in the consumers' comments where the next reader needs it
- [x] 3.2 Tests: §2.2 wiremock ITs (2) — the heal path exercised through the REAL SDK HTTP transport, not a mock trait
- [x] 3.3 Verified: synap_room_selfheal_it 2/2, graph/fulltext/embedder worker suites green, clippy -D warnings clean on cortex-workers
