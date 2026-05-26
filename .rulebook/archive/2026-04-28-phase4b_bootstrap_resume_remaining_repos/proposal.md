# Proposal: phase4b_bootstrap_resume_remaining_repos

## Why

The Cortex plan calls for ingesting **17 Hive repos** but only **3**
have any data in any backend today (snapshot 2026-04-27 22:36 UTC):

```
backend       repos with data
vectorizer    Cortex, Rulebook, Vectorizer
meilisearch   Cortex            (see phase4a for the fan-out fix)
nexus         Cortex, Rulebook, Vectorizer
```

The bootstrap checkpoint
[.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json)
shows only `Cortex` was tracked in the most recent run (started
2026-04-27T21:58:38Z) — `516 files_walked`, `73 commits_walked`,
`589 events_emitted`, `status = done`.

Reading [crates/cortex-bootstrap/src/runner.rs:60](../../../crates/cortex-bootstrap/src/runner.rs#L60)
(`run_repo`) and [config.rs](../../../crates/cortex-bootstrap/src/config.rs)
clarifies the design: `cortex-bootstrap` is **single-repo** by
construction — it loads a `cortex.toml` from the repo root and
walks that one tree. The Rulebook and Vectorizer data already in
the backends came from running `cortex-bootstrap` from those
checkouts in earlier sessions; the data survived but the
checkpoint that tracked them was overwritten.

There is no orchestrator that walks all 17 repos in one invocation,
so the only way to populate them today is to manually `cd` into
each checkout and run `cortex-bootstrap .` — error-prone, easy to
forget, and impossible to validate as a single completion gate.

The 17 Hive repos covered by the plan (per the user's persistent
memory):

```
HiveLLM/Cortex          HiveLLM/Vectorizer       HiveLLM/Nexus
HiveLLM/Synap           HiveLLM/Lexum            HiveLLM/Expert
HiveLLM/Rulebook        HiveLLM/HiveHub          HiveLLM/PonyProtocol
HiveLLM/...             (final list lives in user's docs;
                         this task takes it as input via config)
```

Without all 17 indexed, pre-thinking bundles can never reach the
recall ceiling the architecture targets, and Nexus relationship
queries are limited to a 3-repo subgraph.

## What Changes

- New crate (or `cortex-bootstrap` subcommand)
  `cortex-bootstrap-all` that takes a workspace config listing
  N repos and drives `run_repo` against each — sequentially by
  default, optionally in parallel (`run_repos_parallel` already
  exists at
  [runner.rs:253](../../../crates/cortex-bootstrap/src/runner.rs#L253)).
- Workspace config file `bootstrap.workspace.toml` (or extension
  to `cortex.toml`) shaped as:

  ```toml
  [[repo]]
  id = "Cortex"
  path = "E:/HiveLLM/Cortex"
  config = "cortex.toml"        # optional; defaults to <path>/cortex.toml

  [[repo]]
  id = "Vectorizer"
  path = "E:/HiveLLM/Vectorizer"
  ```

- The orchestrator's checkpoint preserves per-repo progress
  across runs — Ctrl-C in the middle of repo 12 of 17 resumes
  on repo 12, not repo 1.
- Idempotency: every repo run is a no-op when the checkpoint
  reports `status = done` AND `last_git_ref` matches the current
  HEAD of that checkout. A `--force` flag overrides.
- A pre-flight verifier confirms every configured repo path
  exists, is a git checkout, and has a readable `cortex.toml` —
  failures are reported as a block before any walk begins.
- Final report: a single-table summary of all repos with
  events_emitted, files_dropped, duration, and a non-zero exit
  code if any repo failed.

The 14 missing repos are then run in one command, populating
Vectorizer / Meilisearch / Nexus with full coverage.

## Impact

- Affected specs: spec-09 (bootstrap — adds the multi-repo
  orchestration layer; per-repo behaviour is unchanged).
- Affected code:
  - new: `crates/cortex-bootstrap/src/workspace.rs` (or new
    `crates/cortex-bootstrap-all/`) — workspace config parsing +
    orchestration loop
  - `crates/cortex-bootstrap/src/cli.rs` — new `--workspace
    <path>` flag (or new binary entrypoint)
  - `crates/cortex-bootstrap/src/checkpoint.rs` — extend to
    track multiple repos cleanly (already
    `BTreeMap<String, RepoProgress>`, so the shape is ready)
  - new: `bootstrap.workspace.toml` at the repo root checked
    into git as a template (with the 17 repo IDs and example
    paths)
  - tests: integration test with two tiny temp git repos in a
    workspace config, asserts both produce events and the
    checkpoint records both
- Breaking change: NO. Existing single-repo invocations of
  `cortex-bootstrap` continue to work unchanged.
- User benefit: one command populates all 17 repos; resumable
  on interrupt; CI can run it as a smoke gate.

## Source

- Audit data captured 2026-04-27 22:36 UTC.
- Bootstrap state inspected at
  [.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json).
- Single-repo runner confirmed at
  [crates/cortex-bootstrap/src/runner.rs:60](../../../crates/cortex-bootstrap/src/runner.rs#L60).
- Parallel helper available at
  [runner.rs:253](../../../crates/cortex-bootstrap/src/runner.rs#L253).
