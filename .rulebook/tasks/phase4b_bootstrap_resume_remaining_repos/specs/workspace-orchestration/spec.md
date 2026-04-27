# Bootstrap workspace orchestration spec

## ADDED Requirements

### Requirement: Workspace config drives multi-repo bootstrap
The `cortex-bootstrap` CLI SHALL accept a `--workspace <path>` flag pointing at a TOML file that enumerates one or more repos. Each entry MUST carry an `id` (matching the per-repo `cortex.toml` `[cortex] id`) and an absolute `path` to a git checkout. The orchestrator SHALL iterate the configured repos in declaration order and invoke `run_repo` against each.

#### Scenario: two repos in workspace, both bootstrapped
Given a workspace TOML with two entries pointing at two valid git checkouts (`R1` and `R2`)
And neither repo has a prior checkpoint entry
When `cortex-bootstrap --workspace ws.toml` runs to completion
Then both `R1` and `R2` MUST have non-zero `events_published` in the per-repo report
And the checkpoint file MUST contain entries for both `R1` and `R2`
And the final summary table MUST list both repos with `status = done`

#### Scenario: pre-flight aborts when one repo path is missing
Given a workspace TOML with three entries where one path does not exist
When `cortex-bootstrap --workspace ws.toml` runs
Then the orchestrator MUST exit before walking any repo
And the exit code MUST be non-zero
And the failure log MUST list the offending entry id and path

### Requirement: Idempotent resume from checkpoint
The orchestrator SHALL bypass repos whose checkpoint reports `status = done` AND whose `last_git_ref` equals the current `HEAD` of the configured checkout. A `--force` flag overrides the bypass.

#### Scenario: clean resume after Ctrl-C
Given a workspace with three repos where `R1` finished, `R2` is incomplete, and `R3` has not started
When the previous invocation was interrupted between `R2` and `R3`
And the user re-runs `cortex-bootstrap --workspace ws.toml`
Then `R1` MUST be bypassed with an `info` log line citing the matching `last_git_ref`
And `R2` MUST resume (re-running is acceptable; idempotent dedup downstream)
And `R3` MUST run to completion

#### Scenario: --force re-runs a done repo
Given `R1`'s checkpoint reports `status = done` matching current `HEAD`
When `cortex-bootstrap --workspace ws.toml --force` runs
Then `R1` MUST be re-run
And the orchestrator MUST log `info` per forced repo

### Requirement: Single-repo invocations remain unchanged
The existing single-repo invocation form (`cortex-bootstrap <path>` or `cortex-bootstrap` from the repo root) MUST continue to work without a workspace flag. The single-repo code path MUST NOT regress in behaviour or reports.

#### Scenario: backward-compatible single-repo run
Given the user runs `cortex-bootstrap .` from inside a single repo checkout
When the run completes
Then the checkpoint MUST contain exactly one entry for that repo
And the report shape MUST match the prior single-repo output (no new mandatory fields)
