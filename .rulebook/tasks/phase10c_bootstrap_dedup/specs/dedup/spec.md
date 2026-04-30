# Spec: Bootstrap dedup

## ADDED Requirements

### Requirement: Idempotent walker emission

The bootstrap walker MUST consult the `bootstrap_seen` table
before emitting a `cortex.events.bootstrap` envelope. When
`(repo, path)` already has the same `content_hash`, the walker
MUST NOT republish; it MUST refresh `last_run_id` so subsequent
audits can confirm the row was visited.

#### Scenario: re-run on unchanged tree emits nothing
Given the walker has previously bootstrapped `/repo/cortex` with
  every file's hash recorded in `bootstrap_seen`
When the walker re-runs against the same tree
Then zero `cortex.events.bootstrap` envelopes MUST be emitted
And every existing row's `last_run_id` MUST be updated.

#### Scenario: edited file emits exactly once
Given the walker has previously bootstrapped a 100-file repo
When one file's body changes and the walker re-runs
Then exactly one envelope MUST be emitted (the edited file)
And `bootstrap_seen` for that file MUST carry the new hash.

### Requirement: One-shot dedup CLI

`cortex-ops bootstrap dedup [--repo NAME] [--dry-run] [--apply]`
MUST scan the existing lane for duplicate `content_hash` values
within `(repo, path, kind)` tuples, keep the newest ULID, and
delete the duplicates from Vectorizer + Meili + Nexus. Default
mode is `--dry-run`.

#### Scenario: dry-run reports the over-count
Given Vectorizer + Meili + Nexus carry 3 copies of the same ADR
When the operator runs `cortex-ops bootstrap dedup --dry-run`
Then the report MUST list 2 candidates per ADR
And the live indexes MUST stay unchanged.

#### Scenario: apply is idempotent
Given the dry-run reported 24 duplicate decisions to drop
When the operator runs `cortex-ops bootstrap dedup --apply`
Then the lane MUST hold exactly 1 copy per `content_hash`
And a second `--apply` MUST report zero candidates.

### Requirement: Pre-flight warning when re-bootstrapping a dirty lane

When the walker starts and `bootstrap_seen` is empty AND the
existing lane carries more than 2× the disk file count for
`:Decision`, `:Law`, or `:Analysis`, the walker MUST emit a
`cortex.warnings` event with `kind="bootstrap.likely_duplicates"`
suggesting the operator run `cortex-ops bootstrap dedup` first.

#### Scenario: warning fires on a dirty lane
Given disk has 12 laws but the lane has 37
When the walker boots with an empty `bootstrap_seen`
Then exactly one `bootstrap.likely_duplicates` warning MUST be
  emitted referencing the law over-count.
