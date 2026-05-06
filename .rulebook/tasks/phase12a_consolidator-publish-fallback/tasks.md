## 1. Failure logging
- [ ] 1.1 Add `tracing::error!` on every failure path in `publish_consolidation()` with structured fields `event_id`, `query_id`, `reason`, `cortex_ingestion_url`.
- [ ] 1.2 Detect `CORTEX_INGESTION_URL` unset at startup and emit one WARN at boot listing the consequences.

## 2. JSONL fallback
- [ ] 2.1 On any publish failure, append the envelope as a single JSON line to `${CORTEX_HOME}/consolidations.jsonl` (atomic O_APPEND).
- [ ] 2.2 Rotate the file at 100 MB or daily, whichever first. Keep the last 7 rotations.
- [ ] 2.3 Log the absolute path on first append per process.

## 3. Metrics
- [ ] 3.1 Register `cortex_consolidator_publish_failures_total{reason}` counter in `consolidator/metrics.rs`.
- [ ] 3.2 Increment per failure path. Add a regression test that exercises each reason once.

## 4. Replay tool
- [ ] 4.1 Add `cortex-ops consolidations-replay --from <jsonl> [--dry-run]` that POSTs each line to the resolved `CORTEX_INGESTION_URL`.
- [ ] 4.2 Drop lines whose `event_id` already exists in Synap (idempotent replay). Track `replayed`, `dropped`, `errored` counters in stdout.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/12-consolidator.md` § Publishing and `CHANGELOG.md` `[Unreleased]` Added/Fixed.
- [ ] 5.2 Tests: §3.2 metrics regression + JSONL append IT + replay smoke (dry-run + live).
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test -p cortex-workers consolidator -p cortex-cli` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
