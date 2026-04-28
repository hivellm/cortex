# Spec: CI smoke workflow

## ADDED Requirements

### Requirement: full-stack boot in CI

The `health-smoke` GitHub Actions workflow MUST boot a real Cortex
stack (cortex-ingestion, cortex-api, cortex-adapter-claude-code, plus
the cortex-ci-stubs for external dependencies) before running any
smoke test. In-process fakes are insufficient — the 2026-04-28 JSON
truncation bug only manifested over a real named pipe.

The workflow MUST run on both `windows-latest` and `ubuntu-latest`.

#### Scenario: workflow boots the stack
Given a PR is opened
When the `health-smoke` workflow runs
Then `cortex-api`, `cortex-ingestion`, `cortex-adapter-claude-code`,
     and `cortex-ci-stubs` MUST all be running by the time
     `cortex-doctor canary` is invoked.

### Requirement: doctor checks gate the PR

The workflow MUST invoke each of the following and fail the job on
any non-zero exit:
1. `cortex-doctor canary --hook=PostToolUse` (phase8f).
2. `cortex-doctor health` (phase8a CLI).
3. `cortex-doctor config` (phase8d CLI).
4. `cortex-doctor versions` (phase8c CLI).

#### Scenario: regression in PostToolUse fails the PR
Given a PR introduces a regression that drops PostToolUse silently
When the `health-smoke` workflow runs
Then `cortex-doctor canary` MUST exit with code 2
     AND the workflow MUST fail
     AND the PR MUST NOT be mergeable until the regression is fixed.

### Requirement: log artifacts on failure

When any step fails, the workflow MUST upload cortex-api,
cortex-adapter-claude-code, and cortex-ingestion log files as a
named artifact so the PR author can download and inspect them
without re-running CI locally.

#### Scenario: failure preserves the logs
Given the canary step fails
When the workflow's `if: failure()` step runs
Then an artifact named `cortex-logs-<os>-<run_id>` MUST be uploaded
     containing at minimum `cortex-api.log`,
     `cortex-adapter.log`, and `cortex-ingestion.log`.

### Requirement: isolation between concurrent runs

The boot helpers MUST honour a `CORTEX_HOME` environment variable
so concurrent CI runs use isolated archive / WAL / state dirs and
do not collide on a shared `~/.cortex` directory.

#### Scenario: parallel runs do not collide
Given two CI jobs start at the same time on the same runner
And each sets a unique `CORTEX_HOME`
When both jobs boot their respective stacks
Then each stack's archive MUST be confined to its own
     `$CORTEX_HOME/archive/`
     AND neither job MUST observe the other's events.

### Requirement: PR template signal

The repository's `.github/PULL_REQUEST_TEMPLATE.md` MUST include a
"Health checks" section with checkboxes for `scripts/health.bat`
and `scripts/canary.bat` outcomes.

This is a soft cultural signal, not a CI gate — the automated
workflow above is the enforced part.

#### Scenario: PR template renders the section
Given a contributor opens a new PR via the GitHub UI
When the PR description is initialized from the template
Then the description MUST contain a "Health checks" section with
     unchecked checkboxes for the two scripts.
