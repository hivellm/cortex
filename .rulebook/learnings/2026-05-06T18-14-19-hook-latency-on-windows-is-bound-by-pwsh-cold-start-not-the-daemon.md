# Hook latency on Windows is bound by pwsh cold-start, not the daemon
**Source**: manual
**Date**: 2026-05-06
**Related Task**: phase11x_cortex-hook-native-shim
**Tags**: performance, windows, hooks, powershell, profiling, claude-code, adapter, phase11x
## Context

The Cortex Claude Code adapter shipped per-event hook shims as
`.sh` (Linux/macOS) and `.ps1` (Windows). On Windows every hook
invocation cost ~730 ms wall-clock, and the project tracked the
slowness as a "daemon round-trip is slow" problem for months.

## Finding (2026-05-06 profiling)

Bench breakdown of a single hook on Windows (release adapter daemon up):

| Component                              | ms    |
|----------------------------------------|-------|
| `pwsh -NoProfile` cold start           | 545   |
| Script body (log appends, JSON build)  |  95   |
| Named-pipe round-trip + daemon work    |  90   |
| **Total per hook**                     | ~730  |

Daemon work is **12 %** of per-hook cost on Windows. The dominant
cost is just spawning PowerShell. PowerShell cold start is a
structural floor; `pwsh -NoProfile -Command "exit 0"` measures
~545 ms by itself. The named-pipe round-trip — including the
daemon's `cortex-api /v1/query` pre-thinking call — is ~90 ms.

## Lesson

When a hook surface is slow on Windows, **profile process spawn
first, daemon work last**. The shape of the latency sets the fix:

- **Process-spawn-bound** (this case): replace the shim with a
  native binary. Rust release bin starts in ~30–50 ms on Windows,
  10× faster than `pwsh`. Phase 11x's `cortex-hook` cut total
  per-hook cost from 730 ms → ~70 ms.
- **Daemon-bound**: optimise the daemon's hot path (caching,
  fewer round-trips, async batching). Premature optimisation here
  while the cost is in `pwsh` is wasted effort.

## Heuristic

For any agent-host integration on Windows that spawns a fresh
shell per event:

1. Measure `<shell> --no-profile -c "exit 0"` first.
2. Subtract that floor from the wall-clock per-event cost.
3. If the floor is >50 % of the total, ship a native bin shim
   before touching the daemon code.

## Evidence

- Baseline: `crates/cortex-adapter-claude-code/benches/baseline-2026-05-06.txt`
- Bin: `crates/cortex-adapter-claude-code/src/bin/cortex-hook.rs`
- Per-turn delta on a 14-hook turn: ~10.2 s → ~0.9 s (= ~9.3 s / turn).

## Anti-pattern

Adding more hooks (extra log writes, env parsing, JSON tweaks)
to a `.ps1` shim looking for the missing 100 ms while the
unavoidable 545 ms `pwsh` cold start eats every win. Don't optimise
inside the shell when the shell itself is the bottleneck.