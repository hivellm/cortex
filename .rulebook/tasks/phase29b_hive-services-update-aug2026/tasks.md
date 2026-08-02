## 1. August-2026 service/SDK update round
- [x] 1.1 Synap SDK 1.0 → 1.3 (workspace pin `synap-sdk = "1.3"` + lockfile 1.3.0); workspace `cargo check --all-targets` clean — zero API breakage.
- [x] 1.2 Synap container 1.0.0 → 1.3.0 (compose pin + pull + recreate); `/health` reports 1.3.0, healthcheck green.
- [x] 1.3 Nexus decision recorded: STAY on 2.5.0/2.5.0 — Docker Hub carries only `3.0.0-alpha` (no stable 3.x) and crates.io has no `nexus-graph-sdk` 3.x; a major may migrate storage, and production data does not move onto an alpha without explicit user instruction. Marker for the future bump: when stable 3.0 + SDK publish, ALSO re-audit rmcp RUSTSEC-2026-0189 (hivellm/nexus#28) which is pinned by nexus-protocol.
- [x] 1.4 Post-restart incident handled: the synap-only recreate wiped its (ephemeral) stream rooms → ~2400 ERROR/min `Room not found` from the still-running consumers (their boot-time declare from 95f32c7 only runs at startup). Consumers restarted → re-declared → zero errors. Structural fix tracked as §2.
- [ ] 1.5 Rebuild the 7 cortex service images (they still embed synap-sdk 1.0 binaries; protocol-compatible today, but images should track the workspace) + `docker compose up -d` + verify pipeline flow (classifier `last_consume_ts_ms` current, no synap errors over 10 min).
- [ ] 1.6 Update `docs/specs/03-local-stack.md` service-map pin (synap 1.0.0 → 1.3.0 row) + CHANGELOG entry for the round.

## 2. Structural: survive synap restarts without room spam
- [ ] 2.1 Runtime room re-declare: when a consume/publish in `synap_worker::runtime` (and the per-worker consumers) fails with the `Room not found` signal, call `get_or_create_room` once and retry the operation before surfacing the error — bounded (once per room per backoff window) so a genuinely broken room name cannot loop.
- [ ] 2.2 Unit test: consumer whose first `next_batch` errors `Room not found` → re-declare fires → retry succeeds; a second distinct error does NOT re-declare (bound respected).

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
