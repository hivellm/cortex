# Proposal: phase14e_pre-thinking-circuit-breaker

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-003 + F-004 (both CRITICAL).

## Why

Pre-thinking's fail-open path is correct in principle but silently catastrophic in practice. When `cortex-api` is down or slow, every `pipeline.run()` returns `{ fail_open: true, bundle: "" }`, the model proceeds with no context, and nobody notices. There is no circuit breaker, no `fail_open_count` metric, no alert. The empty bundle is indistinguishable from "no relevant context found", so the model has no signal that retrieval failed.

## What Changes

- Add a circuit breaker in `pipeline.run()`: 5+ fail-opens within 60s flips the breaker to OPEN; subsequent calls short-circuit to fail-open instantly without waiting for the timeout.
- Add metric `cortex_pre_thinking_fail_open_total{reason}` with reasons `timeout`, `network`, `unauthorised`, `internal`, `breaker_open`.
- Inject a structured `<!-- cortex: timeout reason=<reason> -->` HTML comment into the empty bundle on fail-open so the model can distinguish outage from genuinely-empty results.
- Doctor check `cortex-ops doctor pre-thinking` reports current breaker state + last-hour fail-open count.
- Alert hook: when the counter increments in 60s, log a structured WARN that scrapes can pick up.

## Impact

- Affected specs: `docs/specs/12-pre-thinking-injection.md` § Fail-open contract + § Circuit breaker.
- Affected code: `crates/cortex-pre-thinking/src/{pipeline.rs,metrics.rs,breaker.rs}` (new module), `crates/cortex-pre-thinking/src/formatter.rs` (timeout sentinel), `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO at the wire format; the bundle gains a sentinel comment when fail-open.
- User benefit: outages are visible; models stop proceeding silently with empty context.
