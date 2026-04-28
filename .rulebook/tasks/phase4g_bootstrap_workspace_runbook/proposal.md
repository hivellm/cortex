# Proposal: phase4g_bootstrap_workspace_runbook

## Why

`phase4b_bootstrap_resume_remaining_repos` shipped the workspace TOML
config + `--workspace` CLI flag + pre-flight verifier + skip-when-done
idempotency + summary table for `cortex-bootstrap`. The actual
authoring of the user's `bootstrap.workspace.toml` against the 17
HiveLLM checkouts on the operator machine, and the live run that
populates Vectorizer / Meilisearch / Nexus, are operational steps —
they need the user's local repo paths and a multi-minute walk against
the live stack, not a code change.

## What Changes

- A template `bootstrap.workspace.toml` checked into the repo root
  with the 17 expected `[[repo]]` entries (paths set to a
  documented placeholder pattern like
  `${HIVE_ROOT}/<RepoName>` so operators search-replace one
  variable instead of editing 17 lines).
- A runbook document `docs/operations/bootstrap-workspace.md`
  with the authoring + run sequence:
  1. Clone the 17 repos under `$HIVE_ROOT`.
  2. Copy the template to `bootstrap.workspace.toml` and replace
     `${HIVE_ROOT}`.
  3. Run `cortex-bootstrap --workspace bootstrap.workspace.toml --estimate` to size the work.
  4. Run `cortex-bootstrap --workspace bootstrap.workspace.toml` for the live walk.
  5. Verify in Vectorizer that every repo has `code` and `docs`
     collections; in Meili that the matching `cortex-{slug}-{family}`
     indexes are populated.

## Impact

- Affected specs: spec-09 (cross-references the runbook).
- Affected code: none (the orchestrator is feature-complete from
  phase4b). New files: `bootstrap.workspace.toml.example` (root)
  + `docs/operations/bootstrap-workspace.md`.
- Breaking change: NO.
- Depends on: phase4b (the orchestrator must be merged before
  this runbook is useful).
- User benefit: closes the audit gap from 2026-04-27 22:36 UTC
  where only 3 of the 17 planned repos had vector / graph
  coverage. After this runbook, recall against
  `Synap/Lexum/Expert/HiveHub/PonyProtocol` and the rest is
  parity-equivalent to `Cortex/Vectorizer/Nexus`.

## Source

- Carved out of `phase4b_bootstrap_resume_remaining_repos` items
  4.1–4.3 (workspace authoring + live run) to honour the
  no-orphan protocol after the orchestrator code landed.
