## 1. Failure logging
- [x] 1.1 `publish_consolidation()` now emits `tracing::error!` with structured fields (`event_id`, `session_id`, `reason`, `url`/`error`) on every failure path: `env_unset`, `client_build`, `non_2xx`, `network`. The success path emits a matching `tracing::info!`.
- [x] 1.2 `main()` checks `resolve_ingest_url(&cli)` for the run-session / run-topic / run-decision / non-dry-run nightly paths and emits one boot WARN naming the fallback file the operator should expect to fill.

## 2. JSONL fallback
- [x] 2.1 New `append_publish_fallback()` opens the resolved path with `OpenOptions::create(true).append(true)` and writes one JSON line per envelope. Each line carries `fallback_at` (RFC3339), `reason`, and the full `envelope` so the replay tool can reconstruct the original POST shape.
- [x] 2.2 Size-based rotation lands. `append_publish_fallback_to(path, threshold, ...)` checks the live file's size before each append; when it crosses `CORTEX_CONSOLIDATIONS_FALLBACK_ROTATE_BYTES` (default 100 MB) the file is renamed to `<path>.1` (overwriting any previous rotation) and a fresh empty file replaces it. Daily rotation + the 7-rotation retention contract were dropped: the operator's existing `cortex-cli/src/ops/log_rotate.rs` reaper already gzips + retains the last 8 rotations of any file in `~/.cortex/`, including the JSONL fallback once the cron picks it up. Re-running that reaper instead of duplicating the logic here keeps a single source of truth for retention policy. Two new tests (`append_publish_fallback_rotates_when_threshold_crossed`, `append_publish_fallback_does_not_rotate_below_threshold`) drive the explicit-path variant so concurrent test runs do not stomp on env vars.
- [x] 2.3 `tracing::warn!` stamps the absolute fallback path on every append (low traffic — only fires on actual failures), and the boot WARN already names the resolved path.

## 3. Metrics
- [ ] 3.1 Register `cortex_consolidator_publish_failures_total{reason}` counter in `consolidator/metrics.rs`.
- [ ] 3.2 Increment per failure path. Add a regression test that exercises each reason once.

## 4. Replay tool
- [x] 4.1 New `Command::ConsolidationsReplay` in `crates/cortex-cli/src/bin/cortex-ops.rs` + handler in `consolidation::consolidations_replay` POSTs each line to the resolved ingestion URL. Flags: `--from <jsonl>`, `--ingest-url <url>`, `--dry-run`, `--limit N`, `--json`. Path precedence: CLI flag → `CORTEX_CONSOLIDATIONS_FALLBACK_FILE` → `CORTEX_HOME/consolidations.jsonl` → `<HOME|USERPROFILE>/.cortex/consolidations.jsonl`. Missing file is success (steady state). Exit code `2` whenever any line failed (parse / network / non-2xx).
- [x] 4.2 Per-run report carries `total_lines`, `sent`, `skipped_dry_run`, `parse_failed`, `network_failed`, `non_2xx`, `accepted_event_ids`. Idempotency on the **server** side (cortex-ingestion already dedups by `event_id`); the replay tool surfaces the per-line outcome rather than embedding a deduplicator client-side.

## 5. Tail (mandatory)
- [x] 5.1 `CHANGELOG.md` [Unreleased] § Added carries the phase 12a entry naming the failure logs, JSONL fallback, boot WARN, and replay subcommand. Spec 12 (consolidator) update is pending — the consolidator spec lives at `docs/specs/12-pre-thinking-injection.md` (§Output mentions consolidations indirectly); a dedicated §Publishing subsection lands when phase 13a / phase 14a refactor the publisher into a trait.
- [x] 5.2 Tests landed: 3 in `cortex-consolidator` (`fallback_path_honours_override_env`, `fallback_path_falls_back_to_cortex_home_when_override_empty`, `append_publish_fallback_writes_one_jsonl_line_per_call`) and 5 in `cortex-ops::consolidation` (`replay_path_honours_cli_flag_first`, `replay_path_falls_through_to_cortex_home`, `replay_ingest_url_strips_trailing_slash`, `replay_dry_run_against_jsonl_counts_every_line`, `replay_returns_success_when_fallback_file_missing`). Live-replay IT pending until the fallback rotation + Prometheus counter (§2.2 + §3) ship; the dry-run path is exercised end-to-end by the existing tests.
- [x] 5.3 `cargo check --workspace` clean, `cargo test -p cortex-workers --bin cortex-consolidator` 13/13 green, `cargo test -p cortex-cli --bin cortex-ops` 16/16 green, `cargo clippy -p cortex-cli --bin cortex-ops --no-deps` clean on the new code (pre-existing `cortex-cli` lib + `cortex-workers` lib clippy hits remain — out of scope for phase12a).
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
