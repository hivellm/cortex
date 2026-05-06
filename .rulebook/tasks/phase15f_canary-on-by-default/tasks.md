## 1. Default flip
- [ ] 1.1 In `cortex_config::CanaryConfig`, default `enabled = true`.
- [ ] 1.2 Document the change in `docs/specs/12-pre-thinking-injection.md` § Canary.
- [ ] 1.3 Migration test: existing `cortex.toml` without the field loads with canary enabled.

## 2. Schedule + alert
- [ ] 2.1 Canary loop ticks every 60s, calls `cortex-api /v1/health/pre-thinking`, records the result.
- [ ] 2.2 On 2 consecutive failures, emit `tracing::warn!(target = "canary", consecutive_failures, last_error, "canary alarm")` for scrape pickup.
- [ ] 2.3 Reset the consecutive counter on first success.

## 3. Storage + dashboard
- [ ] 3.1 New SQLite table `canary_runs { ts, status, latency_ms, error_message }`. Migration in `cortex-storage::metadata`.
- [ ] 3.2 Canary writes one row per tick.
- [ ] 3.3 Cleanup: keep only the last 24h of rows; older rows trimmed each tick.
- [ ] 3.4 New dashboard view `Canary` showing the last 24h as a sparkline + the most recent failure (if any).

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/12-pre-thinking-injection.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: §1.3 + §2 alarm IT + §3.4 GUI snapshot.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C gui test` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
