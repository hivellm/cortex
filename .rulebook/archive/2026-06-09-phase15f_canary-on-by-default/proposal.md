# Proposal: phase15f_canary-on-by-default

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-014 (HIGH).

## Why

The canary path that pings `cortex-api /v1/health` on a fixed cadence is opt-in (`CORTEX_CANARY_ENABLED=1`) and off by default. Operators don't enable it because the flag is undocumented. Result: pre-thinking degradation goes undetected for hours before the user notices.

## What Changes

- Default `CORTEX_CANARY_ENABLED=1` in `cortex_config::CanaryConfig`.
- Schedule every 60s. On 2 consecutive failures, log a structured WARN that the alert pipeline picks up.
- Canary writes a `canary_runs { ts, status, latency_ms, error_message }` row per invocation.
- Dashboard `Canary` view shows the last 24h of runs.

## Impact

- Affected specs: `docs/specs/12-pre-thinking-injection.md` § Canary.
- Affected code: `crates/cortex-pre-thinking/src/canary.rs`, `crates/cortex-storage/src/metadata.rs` (new table), `gui/src/views/Canary.tsx` (new).
- Breaking change: NO. Operators who explicitly set `CORTEX_CANARY_ENABLED=0` keep working.
- User benefit: pre-thinking outages caught within 2 minutes, not hours.
