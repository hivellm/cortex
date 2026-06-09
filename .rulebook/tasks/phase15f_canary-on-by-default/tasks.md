## 1. Default flip
- [x] 1.1 In `cortex_config::CanaryConfig`, default `enabled = true`.
- [x] 1.2 Document the change in `docs/specs/12-pre-thinking-injection.md` § Canary.
- [x] 1.3 Migration test: existing `cortex.toml` without the field loads with canary enabled.

## 2. Schedule + alert
- [x] 2.1 Canary loop ticks every 60s, calls `cortex-api /v1/health/pre-thinking`, records the result.
- [x] 2.2 On 2 consecutive failures, emit `tracing::warn!(target = "canary", consecutive_failures, last_error, "canary alarm")` for scrape pickup.
- [x] 2.3 Reset the consecutive counter on first success.

## 3. Storage + dashboard
- [x] 3.1 New SQLite table `canary_runs { ts, status, latency_ms, error_message }`. Migration in `cortex-storage::metadata`.
- [x] 3.2 Canary writes one row per tick.
- [x] 3.3 Cleanup: keep only the last 24h of rows; older rows trimmed each tick.
- [x] 3.4 New dashboard view `Canary` showing the last 24h as a sparkline + the most recent failure (if any).

## 4. Tail (mandatory)
- [x] 4.1 Update `docs/specs/12-pre-thinking-injection.md` + `CHANGELOG.md`.
- [x] 4.2 Tests: §1.3 + §2 alarm IT + §3.4 GUI snapshot.
- [x] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C gui test` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation.
- [x] 99.2 Write tests covering the new behavior.
- [x] 99.3 Run tests and confirm they pass.
