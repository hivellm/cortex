## 1. Failure logging
- [x] 1.1 `publish_consolidation()` now emits `tracing::error!` with structured fields (`event_id`, `session_id`, `reason`, `url`/`error`) on every failure path: `env_unset`, `client_build`, `non_2xx`, `network`. The success path emits a matching `tracing::info!`.
- [x] 1.2 `main()` checks `resolve_ingest_url(&cli)` for the run-session / run-topic / run-decision / non-dry-run nightly paths and emits one boot WARN naming the fallback file the operator should expect to fill.

## 2. JSONL fallback
- [x] 2.1 New `append_publish_fallback()` opens the resolved path with `OpenOptions::create(true).append(true)` and writes one JSON line per envelope. Each line carries `fallback_at` (RFC3339), `reason`, and the full `envelope` so the replay tool can reconstruct the original POST shape.
- [ ] 2.2 Rotate the file at 100 MB or daily, whichever first. Keep the last 7 rotations.
- [x] 2.3 `tracing::warn!` stamps the absolute fallback path on every append (low traffic — only fires on actual failures), and the boot WARN already names the resolved path.

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
