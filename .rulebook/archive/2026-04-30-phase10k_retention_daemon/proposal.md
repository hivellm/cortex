# Proposal: phase10k_retention_daemon

## Why

Phase9a→9k built every retention sweep + the cron registry that
binds them, but no long-running process actually ticks the scheduler.
`seed_defaults` is only called when the operator types
`cortex-ops schedule init`, and `tick()` only runs when they type
`cortex-ops schedule tick`. The result, observed today: hot data
keeps piling up in `cortex.turn.fp32`, `cortex.tool_call.fp32`, and
the meili / CAS / metadata stores — sweeps never fire on their own.

The cortex-api daemon is the only Cortex process always running on
the host. The fix: have it own the tick loop. Wire it the same way
`silent-drop` already runs (a `tokio::spawn` background task that
pulls due rows every 30 s and shells out to the operator-facing
subcommand).

This requires moving the scheduler module out of `cortex-cli` (a
binary-side crate that depends on `cortex-api`) and into
`cortex-retention` (which `cortex-api` can safely depend on). The
move is mechanical; the public API stays identical, callers update
imports with a one-line rewrite.

## What Changes

1. Move `crates/cortex-cli/src/ops/scheduler.rs` →
   `crates/cortex-retention/src/scheduler.rs`. Identical surface:
   `Runner` trait, `ProcessRunner`, `MemoryRunner`, `tick`,
   `run_now`, `seed_defaults`, `parse_schedule`, `next_after`,
   plus the `STREAM_CAP_BYTES` / `DEFAULT_TICK_INTERVAL_SECS`
   constants and the `RunOutcome` / `RunError` types.
2. `cortex-cli` re-exports the module path so `cortex-ops`'s
   `schedule` subcommand keeps working without a touch to the bin
   (one-line `pub use` in `ops/mod.rs`).
3. Add `cortex-retention` as a `cortex-api` dep + the `cron` /
   `chrono` workspace deps where missing.
4. New `cortex-api/src/retention_daemon.rs` — background task that:
   - Calls `seed_defaults` once on boot.
   - Loops `tick()` every 30 s using `ProcessRunner` so the spawned
     subprocesses (`cortex-ops retention-sweep`, etc.) inherit the
     daemon's PATH.
   - Honours `CORTEX_RETENTION_DAEMON=disabled` so an operator
     running the daemon under a CI without a metadata DB can opt out.
5. `cortex-api/src/main.rs` — wire the new daemon next to
   `silent-drop`, gated on `metadata.is_some()`.

## Impact

- Affected specs: `docs/specs/19-retention.md` §scheduler (clarify
  who runs the tick), NEW `cortex-api/src/retention_daemon.rs` doc
  comment.
- Affected code: `crates/cortex-retention/`,
  `crates/cortex-cli/src/ops/`, `crates/cortex-api/src/`.
- Breaking change: NO. CLI surface unchanged; public scheduler API
  preserved through the re-export.
- User benefit: retention sweeps fire automatically. The garbage
  the operator sees today (FP32 records past 30 d, untiered meili /
  CAS / metadata rows) drains on the next nightly tick.
