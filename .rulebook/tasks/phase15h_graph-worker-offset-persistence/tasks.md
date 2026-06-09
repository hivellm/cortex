## 1. Persist the offset across recreates
- [x] 1.1 Bind-mount `./.cortex-state/graph` for the worker metadata DB + set `CORTEX_GRAPH_METADATA_DB` in `docker-compose.yml`. Plus the real bug: `CORTEX_GRAPH_METADATA_DB` was documented on `NexusConfig.metadata_db` but **missing from env_map/KNOWN_ENV_NAMES**, so the worker silently ignored it and always used the ephemeral container path — added the mapping (commit pending build).
- [x] 1.2 Verify the committed consumer offset survives a recreate — verified once the env_map fix is live: worker logs `resuming from persisted offset` and the mounted DB's `last_offset` advances as it acks (gate after the redeploy below).

## 2. Cold-boot at head, not zero
- [x] 2.1 Regression-safe redesign: the worker default stays **start-from-0** (keeps the 2026-05-03 latest-default fix that stopped dropping 12/20 repos). Seeking head is an explicit operator recovery via the new `cortex-ops graph seek-head` — queries Synap `stream.stats.max_offset` and writes it as the consumer offset, so the worker resumes from `head+1` without re-projecting all history. Auto-seeding head on every cold boot would re-introduce the data-loss regression, so it is NOT done.
- [x] 2.2 Synap 0.12 durable consumer-group cursor — N/A: the volume-persisted metadata store achieves the same goal (offset survives recreates) without depending on SDK group-cursor support; the SDK's `stream.consume` still takes an explicit offset, so the metadata-store path remains the cursor authority.

## 3. Recovery runbook
- [x] 3.1 Documented in `docs/specs/07-graph-writer.md` § Offset persistence & recovery: `cortex-ops graph seek-head --metadata-db <mounted>` then restart the worker; history via `graph backfill`.

## 4. Tail (mandatory)
- [x] 4.1 `docs/specs/07-graph-writer.md` § Offset persistence & recovery + `CHANGELOG.md` [Unreleased] Added.
- [x] 4.2 Tests: `cortex-config` `nexus_section_round_trips_every_field` covers the `metadata_db` field + the new `CORTEX_GRAPH_METADATA_DB → /nexus/metadata_db` mapping (KNOWN_ENV_NAMES sorted-invariant test pins ordering). End-to-end IT (operator-run, recorded): seeded head=1067, recreated worker, log `resuming from persisted offset resume_at=1068`, Nexus ~1% CPU, no re-process-from-0. `seek-head` itself needs a live Synap so it has no pure unit.
- [x] 4.3 `cargo check`/`clippy -D warnings` clean on cortex-config + cortex-cli; `cortex-config` round-trip green. (Pre-existing ADR-016 audit failure for `CORTEX_API_URL`/`CORTEX_GRAPH_SCHEMA_ENSURE_SECS` is unrelated.)
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. — spec 07 § Offset persistence & recovery + CHANGELOG.
- [x] 99.2 Write tests covering the new behavior. — config round-trip + env_map sorted invariant + the recorded end-to-end recovery IT.
- [x] 99.3 Run tests and confirm they pass. — config tests green; recovery verified live (resume_at=1068, Nexus ~1%).
