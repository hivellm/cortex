# Proposal: phase8e_silent_drop_detector

## Why

The flagship 2026-04-28 bug was a *silent* drop: the adapter received
PostToolUse frames, the hook script saw `-> ok` responses, but the
adapter's `handle_pipe` truncated the JSON and quietly returned
`HookResponse::empty`. No envelope built, no envelope published. The
log only had a `WARN: malformed hook frame` for one in a thousand
attempts because the log level was at INFO by default — and even then,
each malformed frame produced one terse warn line that was easily
buried by other traffic.

phase8b (counters per stage) already gives the raw signal. phase8e
adds the *detector*: a thin always-on watcher that computes counter
divergence at every stage transition and raises a structured alert
the moment a sustained drop is observed.

The crucial difference from phase8b: this isn't an aggregation
endpoint you have to remember to call — it's an in-process watcher
that fires its own alert envelope into the cortex-api event lane,
so the alert shows up as a high-severity row in the existing Live
Timeline / Violations views without anyone having to look for it.

## What Changes

1. NEW background task in `cortex-api`: `silent_drop_watcher`. Every
   30 seconds it polls `/v1/health/divergence` (phase8b internal API)
   and for every row whose `severity` transitions from `ok` →
   `warn` or `warn` → `critical`, it:
   - emits an `Envelope { kind: "law_violation", payload: { law_id:
     "silent-drop-<pair>", severity, message, ... } }` published into
     the cortex-ingestion archive AND the cortex-api in-memory
     `MemoryKeywordLane` so the violation surfaces in Live Timeline
     and the existing Violations view immediately;
   - persists the alert state in `~/.cortex/alerts/<pair>.json` so
     repeated alerts dedupe and don't flood the lane.

2. NEW per-pair sensitivity config in `~/.cortex/cortex.toml`:
   ```toml
   [silent_drop]
   enabled = true
   poll_interval_secs = 30
   pairs = [
     { upstream = "adapter.ipc.PostToolUse", downstream = "adapter.publisher.tool_call",
       warn_delta_60s = 10, critical_delta_60s = 50 },
     ...
   ]
   ```
   Sensible defaults shipped in code; toml override only for tuning.

3. NEW alert state machine: each pair has a state ∈ { ok, warn,
   critical }. Transitions:
   - `ok → warn` after 2 consecutive polls show `delta_growth_60s >
     warn_delta_60s` (avoids transient bursts).
   - `warn → critical` after 1 poll shows `delta_growth_60s >
     critical_delta_60s`.
   - Recovery: `* → ok` after 5 consecutive polls show the delta
     not growing.
   Each transition emits exactly one alert envelope.

4. Escalation hooks (optional, gated by config):
   - `silent_drop.webhook_url`: POST the alert to a webhook.
   - `silent_drop.write_to_handoff`: append a line to
     `.rulebook/handoff/_pending.md` so the next session sees it.

5. CLI: `cortex doctor alerts` lists current alerts + their state.

## Impact

- Affected specs: NEW `specs/silent_drop/spec.md`.
- Affected code:
  - NEW `crates/cortex-api/src/health/silent_drop.rs` (background task)
  - `crates/cortex-api/src/main.rs` (spawn the task)
  - NEW `crates/cortex-api/src/health/alerts.rs` (state machine)
  - `~/.cortex/cortex.toml` (NEW [silent_drop] section)
  - cortex-doctor: new `alerts` subcommand
- Depends on: phase8b (divergence endpoint).
- Breaking change: NO (additive watcher).
- User benefit: silent drops become *loud* — the law_violation
  envelope shows up in the existing Violations view with a clear
  narrative ("PostToolUse hooks fired 100x in 60s but only 0
  tool_call envelopes published — adapter is dropping frames").
