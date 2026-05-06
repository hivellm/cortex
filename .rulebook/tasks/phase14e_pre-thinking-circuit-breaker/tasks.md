## 1. Circuit breaker
- [ ] 1.1 New module `crates/cortex-pre-thinking/src/breaker.rs` exposing `Breaker { state: Closed | Open | HalfOpen, fail_count, window_start, threshold, window_secs }`.
- [ ] 1.2 `Breaker::on_fail()` increments. When `fail_count >= threshold` within `window_secs`, transitions to Open. Auto-recovers to HalfOpen after `cooldown_secs`.
- [ ] 1.3 `Breaker::guard(&self) -> Result<Permit, BreakerOpen>` short-circuits when Open.
- [ ] 1.4 Defaults: `threshold = 5`, `window_secs = 60`, `cooldown_secs = 30`. All overridable via Config.
- [ ] 1.5 Unit tests: 6 cases (closed → open via burst, open short-circuits, half-open success → closed, half-open failure → open, window roll-over resets count, threshold parametric).

## 2. Pipeline integration
- [ ] 2.1 `pipeline.run()` wraps the cortex-api call in `Breaker::guard()`. Open → instant fail-open with reason `breaker_open`.
- [ ] 2.2 On any fail-open path, increment `cortex_pre_thinking_fail_open_total{reason}` counter.
- [ ] 2.3 Inject `<!-- cortex: timeout reason=<reason> query_id=<id> -->` into the bundle on fail-open. Empty bundle becomes distinguishable from "no results" by the sentinel presence.

## 3. Doctor + alerting
- [ ] 3.1 `cortex-ops doctor pre-thinking` prints breaker state + last-hour fail-open count by reason.
- [ ] 3.2 Structured WARN log on every breaker state transition for scrape-based alerting.
- [ ] 3.3 Health endpoint `cortex-api /v1/health/pre-thinking` echoes the breaker state.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/12-pre-thinking-injection.md` § Fail-open contract + § Circuit breaker + `CHANGELOG.md`.
- [ ] 4.2 Tests: §1.5 + pipeline IT exercising burst-trigger + sentinel-presence assertion.
- [ ] 4.3 `cargo check --workspace && cargo clippy -p cortex-pre-thinking -- -D warnings && cargo test -p cortex-pre-thinking` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
