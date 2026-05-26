# CI smoke workflow boots real binaries to catch IPC regressions

**Category**: ci
**Tags**: phase8h, ci, smoke, github-actions, cortex

## Description

A GitHub Actions workflow boots the actual cortex-ingestion + cortex-api + cortex-adapter daemons (via reusable boot-stack.{sh,bat} scripts), polls /v1/health for overall ≤ degraded, then runs the existing doctor wrappers (health, doctor-versions, doctor-config, canary) in series. Each runner gets a CORTEX_HOME=${{ runner.temp }}/cortex-home-<run_id>-<attempt> for concurrent-run isolation. On failure the $CORTEX_HOME/logs/ tree uploads as a named artifact for postmortem.

## Example

# .github/workflows/health-smoke.yml
- run: scripts/ci/boot-stack.sh   # waits for /v1/health
- run: scripts/health.sh           # exit ≤ 1 ok
- run: scripts/doctor-versions.sh  # exit 0 ok
- run: cargo run -p cortex-cli --bin cortex-ops -- canary --hook=PostToolUse
- if: failure()
  uses: actions/upload-artifact@v4
  with: { path: $CORTEX_HOME/logs/** }

## When to Use

Multi-binary stacks where the failure mode lives in the IPC / pipe / archive boundary that in-process unit tests can't exercise. The 2026-04-28 JSON truncation bug only manifested over a real named pipe — a boot-real-binaries CI gate is the only way to catch that class before merge.

## When NOT to Use

When external service stubs would meaningfully shape the test (live Vectorizer / Nexus integration). Phase8h skirts that by relying on the Memory* lane fallbacks and accepting overall=degraded as a valid boot state — anything stronger needs a separate workflow with the live services or stubs.
