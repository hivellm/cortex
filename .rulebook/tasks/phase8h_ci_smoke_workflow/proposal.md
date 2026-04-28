# Proposal: phase8h_ci_smoke_workflow

## Why

phase8a–8g detect issues in *production*. phase8h closes the loop:
prevent the issues from reaching production at all by booting the
full Cortex stack inside CI for every PR and running the synthetic
canary plus the doctor checks against it. If the canary times out
or the doctor flags any critical finding, the workflow fails the PR.

Today's existing `cargo test` suite uses in-process fakes
(`MemoryPublisher`, etc.). It would not have caught the JSON
truncation bug because the bug only manifested over a real named
pipe with a real binary. CI must boot the binaries.

## What Changes

1. NEW GitHub Actions workflow `.github/workflows/health-smoke.yml`:
   - Trigger: `pull_request` + `push` to main.
   - Job `health-smoke` (Windows + Linux matrix) that:
     a) Builds the workspace in release mode.
     b) Boots `cortex-ingestion`, `cortex-api`, `cortex-adapter-claude-code`
        in the background with port assignments matching the dev defaults.
     c) Waits for `/v1/health` to report `overall: ok` (max 60 s).
     d) Runs `cortex-doctor canary --hook=PostToolUse` (phase8f).
     e) Runs `cortex-doctor health` (phase8a CLI).
     f) Runs `cortex-doctor config` (phase8d CLI).
     g) Runs `cortex-doctor versions` (phase8c CLI).
     h) Tears down the daemons. Job fails if any of d–g exit non-zero.

2. NEW `scripts/ci/boot-stack.bat` and `boot-stack.sh` reusable boot
   helpers that the workflow calls; they spawn binaries with a
   per-run `CORTEX_HOME` so concurrent CI runs don't collide.

3. Mock external dependencies (Vectorizer, Nexus, Meili, Synap) with
   tiny stub HTTP servers (`crates/cortex-ci-stubs/`) so the smoke
   test doesn't require those services to be online — but real
   integration tests against live services run in a separate
   nightly workflow.

4. NEW workflow `.github/workflows/version-coherence.yml` (referenced
   by phase8c) — implementation lands here so CI gating is one
   coherent piece of work.

5. PR template entry: a checkbox "I ran `scripts/health.bat` locally
   and it returned 0" — a soft cultural reminder that complements
   the automated gate.

## Impact

- Affected specs: NEW `specs/ci_smoke/spec.md`.
- Affected code:
  - NEW `.github/workflows/health-smoke.yml`
  - NEW `.github/workflows/version-coherence.yml`
  - NEW `scripts/ci/boot-stack.bat` / `.sh`
  - NEW `scripts/ci/teardown-stack.bat` / `.sh`
  - NEW `crates/cortex-ci-stubs/` (Vectorizer/Nexus/Meili/Synap stubs)
  - `.github/PULL_REQUEST_TEMPLATE.md` — add health checkbox
- Depends on: phase8a, 8c, 8d, 8f.
- Breaking change: NO (new CI gates only).
- User benefit: regressions like the 2026-04-28 JSON truncation
  cannot reach main; PRs that break the pipeline are caught
  before merge instead of in production.
