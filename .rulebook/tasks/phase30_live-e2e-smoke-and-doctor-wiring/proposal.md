# Proposal: phase30_live-e2e-smoke-and-doctor-wiring

## Why

Source: `docs/analysis/cortex-platform-2026-07/` (README.md +
execution-plan.md; `findings.md` is referenced by the README as the
detailed audit but had not yet been written to disk as of this task's
authoring — a concurrent authoring pass in the same working tree). Per
its execution plan, this task's scheduled-smoke scope generalizes that
plan's narrower "dead-wire detection... phantom-link verifier, feedback
loop, cache instrumentation" item and its "extend the 600s freshness
check to graph-worker consume-loop activity" item into one systematic
"registered but never exercised" gate, covering MCP tools as well as
worker consume loops. Rulebook knowledge base entry
`ship-then-dead-wire-features-land-unit-tested-but-disconnected-from-the-live-path`
(tagged `analysis:cortex-platform-2026-07`, captured 2026-07-05) records
"ship-then-dead-wire" as a recurring failure mode in
this project: the phantom-link verifier landed dead-wired and was only
connected at boot in a later phase; pre-thinking cache counters were
invisible cross-process; the adapter daemon was simply not running while
everything else looked fine. The same pass found a fresh, live instance
during manual testing, detailed in the companion entry
`container-healthcheck-passing-does-not-mean-the-worker-s-consume-loop-is-progressing`:
`cortex-graph-worker`'s Nexus-consumer loop silently stopped processing on
2026-06-27 and stayed dead for 8 days while Docker's container HEALTHCHECK
kept reporting it "healthy" throughout, because that probe only checks that
`/healthz` responds, not that the work loop is progressing.

Notably, detection logic for this class of problem already exists:
`crates/cortex-workers/src/admin_health.rs`'s `rules::freshness_state()`
(constant `DEFAULT_FRESHNESS_DEGRADED_SECS = 600`, bumped from 120 per its
own test comment) correctly flagged the stalled worker `Degraded` once
checked — cortex-api's own `/v1/health` freshness signal disagreed with
Docker's container-level healthcheck, and the freshness signal was the one
telling the truth. The gap is not detection capability; it is that nothing
runs this check on a schedule against a stack that has been up for days, and
nothing alerts when it flips to `Degraded` — the stall was only caught by a
human doing manual testing, 8 days late.

The existing CI gates cannot close this gap structurally, not just by
oversight: `.github/workflows/health-smoke.yml` (phase8h), `.github/workflows/doctor.yml`
(phase4j doctor-consistency), and `.github/workflows/retention-canary.yml`
(phase9j) each boot a **fresh** stack per run and each explicitly documents,
in its own YAML comments, that its nightly `schedule:` trigger was removed
because a freshly-booted CI stack was non-deterministic and the schedule
"failed every night and flooded the maintainer's inbox." A worker that
stalls only after several days of uptime cannot be caught by a gate that
tears the stack down and rebuilds it every run — this task must target a
stack that stays up between checks, not reproduce the fresh-boot pattern
that already failed once for this exact reason.

Two operator doctor scripts (`bin/cortex-doctor` / `bin/cortex-doctor.ps1`,
and the Windows-native `curl -o /dev/null` bug in
`crates/cortex-cli/src/bin/cortex-ops/doctor.rs`, also confirmed in the
knowledge base as `operator-cli-tools-shelling-out-to-curl-o-dev-null-break-on-native-windows-hosts`)
were found broken during this same manual pass; those specific fixes are
tracked in `phase28_live-testing-bugfixes` (referenced, not duplicated,
here) — this task is about the systemic gap that let all of the above go
unnoticed for days to months, not about the individual bugs.

## What Changes

1. NEW scheduled (nightly, cron-triggered) GitHub Actions workflow running
   the full doctor suite (health, doctor-versions, doctor-config, canary)
   plus one real end-to-end retrieval smoke query (`cortex_query` /
   `cortex_pre_thinking`), targeting a long-lived stack instance that has
   been running for longer than a CI-boot window — not the fresh-boot
   pattern the three existing gates deliberately stopped scheduling.
2. NEW "registered but never exercised" check, generalizing the
   ship-then-dead-wire lesson into an automated gate:
   - MCP tools: add per-tool last-invoked tracking (does not exist today —
     `crates/cortex-mcp-server/src/tools.rs`'s `ToolContext`/`ToolRegistry`
     carry no invocation counters) and fail the scheduled run if any
     registered tool has gone silent within a defined window.
   - Worker consume loops: extend and schedule the freshness check that
     already exists in `admin_health.rs` (600-second no-activity threshold)
     against the long-lived stack, with alerting on the transition to
     `Degraded` — the detection logic is not new, the schedule and the
     alert are.
3. Extend `doctor-versions` / `doctor-config` coverage to include
   `cortex-reranker` (a third-party TEI image with no Docker Compose
   healthcheck at all today — version-drift checking does not apply to it
   the way it does to cortex-built binaries, but a healthcheck and
   config-coherence check do) and `cortex-adapter-claude` (a cortex-built
   binary that runs host-side outside docker-compose and is therefore easy
   to omit from the known-binary list `doctor-versions` walks).
4. Document the scheduled workflow and its alerting path — where a nightly
   failure surfaces (GitHub Actions run notification at minimum; decide and
   document whether it also reaches a dashboard, Slack, or an issue) — in
   `docs/architecture.md` §13 (Observability), following the existing
   §13.5–§13.12 numbering, or a new `docs/cortex/` runbook if scope outgrows
   one section.

## Impact

- Affected specs: NEW `specs/observability/spec.md` (this task); MODIFIED
  `docs/architecture.md` §13 (new subsection documenting the scheduled
  workflow).
- Affected code: NEW `.github/workflows/*.yml` (scheduled doctor+smoke),
  `crates/cortex-mcp-server/src/tools.rs` / `server.rs` (per-tool invocation
  tracking), `crates/cortex-workers/src/admin_health.rs` (scheduled use +
  alerting hook), `crates/cortex-cli/src/bin/cortex-ops/doctor.rs` and
  `doctor_synap_workers.rs` (extend known-service coverage),
  `docker-compose.yml` (add `cortex-reranker` healthcheck).
- Breaking change: NO — additive CI/observability infrastructure only.
- User benefit: closes the exact gap that let `cortex-graph-worker` sit
  silently dead for 8 days, and generalizes the four confirmed
  ship-then-dead-wire instances (phantom-link verifier, pre-thinking cache
  counters, adapter daemon, graph-worker) into one automated
  registered-surface-freshness gate instead of relying on the next incident
  being caught by manual testing.
