## 1. Scheduled doctor + e2e smoke against a long-lived stack
- [ ] 1.1 Designate a long-lived stack target for the scheduled run,
      distinct from the fresh-boot-then-teardown pattern `health-smoke.yml`
      / `doctor.yml` / `retention-canary.yml` use — all three explicitly
      removed their nightly schedules because a freshly-booted CI stack is
      non-deterministic, so this task must not reproduce that same problem
      under a new name.
- [ ] 1.2 NEW `.github/workflows/scheduled-doctor-smoke.yml` with a
      `schedule:` cron trigger (nightly) running `doctor health`,
      `doctor-versions`, `doctor-config`, `canary`, plus one real end-to-end
      `cortex_query` / `cortex_pre_thinking` smoke call against the
      long-lived instance.
- [ ] 1.3 Wire failure reporting for the scheduled run (surfaced per §4).

## 2. "Registered but never exercised" gate
- [ ] 2.1 Add per-MCP-tool last-invoked tracking in
      `crates/cortex-mcp-server/src/tools.rs` (`ToolContext` /
      `ToolRegistry` carry no invocation counters today) so every
      registered tool has a queryable last-called timestamp.
- [ ] 2.2 Extend the scheduled run to assert every registered MCP tool was
      called at least once within a defined recent window (e.g. 7 days);
      fail — not just warn — on any tool that has gone silent.
- [ ] 2.3 Extend the scheduled run to assert every worker's consume loop
      shows recent activity, building on the freshness check that already
      exists (`crates/cortex-workers/src/admin_health.rs::rules::freshness_state`,
      `DEFAULT_FRESHNESS_DEGRADED_SECS = 600`) — the gap is scheduling and
      alerting against a long-lived instance, not the detection logic
      itself, which already correctly flagged the graph-worker stall as
      `Degraded` once checked.
- [ ] 2.4 Confirm this generalizes the four confirmed ship-then-dead-wire
      instances (phantom-link verifier, pre-thinking cache counters,
      adapter daemon, graph-worker) into one gate, rather than a one-off
      fix scoped only to the graph-worker case.

## 3. Doctor coverage gaps — cortex-reranker and cortex-adapter-claude
- [ ] 3.1 Add a `healthcheck:` block to `docker-compose.yml`'s
      `cortex-reranker` service (currently has none, unlike `nexus` /
      `synap` / `meilisearch`) against TEI's own `/health` endpoint.
- [ ] 3.2 Extend `doctor-config` coherence checks to cover
      `cortex-reranker`'s `CORTEX_RERANKER_ENDPOINT` wiring — version-drift
      checking does not apply the same way since it is a third-party TEI
      image, not a cortex-built binary.
- [ ] 3.3 Add `cortex-adapter-claude` (host-side, outside docker-compose,
      reached via `CORTEX_ADAPTER_ADMIN_URL`) to `doctor-versions`' known-binary
      list — it is a cortex-built binary with a meaningful git-SHA drift
      check, and is easy to forget precisely because it runs outside the
      compose-managed fleet.

## 4. Document the workflow and its alerting path
- [ ] 4.1 Add `docs/architecture.md` §13.13 "Observability — scheduled
      long-lived-stack doctor (phase30)", following the existing
      §13.5–§13.12 numbering convention, or a new `docs/cortex/` runbook if
      scope outgrows a single section.
- [ ] 4.2 Document exactly where a nightly failure surfaces (GitHub Actions
      run failure notification at minimum) and explicitly decide and
      document whether it also posts to a dashboard, Slack, or an issue —
      do not leave this undecided in the doc.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation —
      `docs/architecture.md` §13.13 and CHANGELOG.md.
- [ ] 5.2 Write tests covering the new behavior — unit tests for the
      "registered but never exercised" logic (inject a stale
      last-invoked/last-activity timestamp, assert the check fails);
      integration test for the `doctor-versions` / `doctor-config`
      extensions covering `cortex-reranker` and `cortex-adapter-claude`.
- [ ] 5.3 Run tests and confirm they pass — including a manual
      `workflow_dispatch` dry-run of the scheduled workflow before relying
      on the cron trigger.
