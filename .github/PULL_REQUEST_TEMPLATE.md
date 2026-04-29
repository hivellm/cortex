<!--
Phase8h — PR template carries a Health checks block as a soft
cultural signal that complements the automated `health-smoke`
workflow. Tick whichever local checks you ran; the workflow is
the enforced gate.
-->

## Summary

<!-- 1–3 bullets summarising the change. -->

-

## Test plan

<!-- Checklist of how you validated the change locally. -->

- [ ]

## Health checks

Phase8h soft signal — tick whichever local checks ran clean. The
[`health-smoke` workflow](./.github/workflows/health-smoke.yml) is
the enforced gate; these checkboxes raise author awareness.

- [ ] `scripts/health.{bat,sh}` returned exit code 0 or 1
      (`ok` / `degraded` accepted; `down` / unreachable need a
      narrative).
- [ ] `scripts/doctor-versions.{bat,sh}` returned exit code 0
      (no version drift between running binaries and HEAD).
- [ ] `scripts/doctor-config.{bat,sh}` returned exit code 0 or 1
      (no critical findings).
- [ ] `scripts/canary.{bat,sh}` returned exit code 0 (synthetic
      end-to-end frame round-tripped within the deadline) — N/A
      when the change does not touch the adapter / ingestion path.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
