# Spec: cortex-workers consolidation host

## ADDED Requirements

### Requirement: Worker-style daemons live in cortex-workers

The workspace SHALL host every worker-style daemon library and
binary inside `cortex-workers`. Five previously-separate crates
(`cortex-classifier`, `cortex-ingestion`, `cortex-claude-archive`,
`cortex-consolidator`, `cortex-retention`) MUST be folded into
`cortex-workers/src/<module>/` with their bins moved to
`cortex-workers/src/bin/<bin-name>.rs`.

#### Scenario: workspace member count drops

Given the workspace `Cargo.toml` lists 14 members at task start
When the task ships
Then the workspace `Cargo.toml` MUST list exactly 9 members
And the deleted entries MUST be `cortex-classifier`, `cortex-ingestion`, `cortex-claude-archive`, `cortex-consolidator`, `cortex-retention`

#### Scenario: every merged module is reachable via re-export

Given the merged cortex-workers crate
When an external consumer writes `use cortex_workers::{classifier, ingestion, claude_archive, consolidator, retention};`
Then the import MUST compile against default features for `classifier`, `ingestion`, `consolidator`, `retention`
And `claude_archive` MUST require `--features claude-archive`

### Requirement: Bin names are preserved across the merge

Operator-facing bin names SHALL NOT change. The five
pre-existing bins (`cortex-classifier-worker`,
`cortex-embedder-worker`, `cortex-fulltext-worker`,
`cortex-graph-worker`, `cortex-graph-backfill`) keep their names
as today. The four absorbed bins (`cortex-ingestion`,
`cortex-claude-archive`, `cortex-consolidator`, plus the new
`cortex-retention-sweep`) keep the same operator-facing names.

#### Scenario: each bin produces a runnable binary

Given a clean workspace build
When `cargo build --workspace --all-features --bins` runs
Then each of the 9 bins MUST produce an executable artefact under `target/<profile>/`
And the `--help` output of each bin MUST match the checked-in golden file at `crates/cortex-workers/tests/fixtures/bin_help/<bin-name>.txt`

#### Scenario: required-features gates the claude-archive bin

Given the `claude-archive` feature is OFF
When `cargo build --bin cortex-claude-archive` runs
Then the build MUST fail with the standard cargo `required-features` error
And the error message MUST cite `required-features = ["claude-archive"]`

### Requirement: Heavy deps are feature-gated

The workers crate SHALL NOT pull `notify`, `notify-debouncer-mini`,
`cron`, `flate2`, `zstd`, or `rusqlite` into the default-feature
build. They MUST be gated behind the `claude-archive` feature.

#### Scenario: default build does not link archive deps

Given a build with default features only
When `cargo tree -p cortex-workers --no-default-features --features default` runs
Then the dependency graph MUST NOT contain `notify`, `notify-debouncer-mini`, or `cron` (they belong to the optional `claude-archive` feature)

#### Scenario: claude-archive feature pulls every gated dep

Given a build with `--features claude-archive`
When `cargo tree -p cortex-workers --features claude-archive` runs
Then the dependency graph MUST contain every dep listed under the `claude-archive` feature flag

### Requirement: Sweep parity between CLI subcommand and new bin

The new `cortex-retention-sweep` bin SHALL produce
byte-equivalent audit envelopes to the pre-existing
`cortex-cli sweep` subcommand against the same fixture corpus.
The `cortex-cli sweep` subcommand MUST delegate in-process to
the same `cortex_workers::retention::run_sweep` entry point the
bin uses.

#### Scenario: identical audit envelopes against fixture

Given a fixture corpus and identical CLI flags
When `cortex-cli sweep --dry-run --scope cortex` runs
And `cortex-retention-sweep --dry-run --scope cortex` runs
Then both invocations MUST emit byte-identical audit envelopes ordered by event_id
And both invocations MUST exit with the same status code

#### Scenario: in-process delegation prevents drift

Given the cortex-cli sweep subcommand
When the subcommand executes against any input
Then it MUST call `cortex_workers::retention::run_sweep` directly (no shell-out, no re-implementation)
And the call site MUST be a single line that forwards parsed args to the lib entry point

### Requirement: Module re-exports preserve the public API surface

Every type, trait, function, and constant publicly exported by
the five merged crates SHALL remain reachable through
`cortex_workers::<module>::*` after the merge. Internal-only
items (`pub(crate)`, `pub(super)`) MAY be re-scoped to align
with the new module hierarchy.

#### Scenario: pre-merge symbol set equals post-merge symbol set

Given the union of `pub` items in `cortex-classifier`, `cortex-ingestion`, `cortex-claude-archive`, `cortex-consolidator`, `cortex-retention` at task start
When the task ships
Then every item in that union MUST be reachable as `cortex_workers::<module>::<name>` (where `<module>` is the matching merged module name)
And the `module_re_export_it` test MUST verify the contract by referencing each item via the new path

### Requirement: Build verification gates each phase

Each of the five phases (§1-§5) SHALL ship green: `cargo check`
zero errors, `cargo test` 100 % pass on the affected crates,
`cargo clippy --no-deps --tests -- -D warnings` zero new
warnings. Old crate directories MUST be deleted only after the
phase's verify step passes.

#### Scenario: phase ships green or rolls back

Given any §N implementation step (1 ≤ N ≤ 5)
When the verify item (§N.last) runs
Then it MUST execute `cargo check`, `cargo test`, and `cargo clippy` for the affected crates
And on any failure the old crate directory MUST remain in place (no delete)
And the failure MUST be diagnosed and fixed inside §N before proceeding to §N+1

#### Scenario: workspace-wide gate at tail

Given §6.3 (the workspace-wide verify gate)
When `cargo check --workspace --all-features` runs
Then it MUST exit zero
And `cargo test --workspace` MUST report 100 % pass
And `cargo clippy --workspace --no-deps --tests -- -D warnings` MUST report zero warnings
And `cargo build --workspace --all-features --bins` MUST produce all 9 expected binaries
And `cargo build --workspace --bins` (default features) MUST produce 8 binaries (every bin except `cortex-claude-archive` which is feature-gated)

### Requirement: Build-time delta is measured empirically

The task SHALL record the cold-build time for the workspace
before and after the merge. Both numbers MUST land in the
§6.4 learning entry so the build-time impact is empirical.

#### Scenario: cold-build time is recorded both pre and post

Given the §6.4 learning entry
When an operator reads it
Then they MUST find two timestamped measurements: one pre-merge from a clean target and one post-merge from a clean target
And both measurements MUST cite the host (CPU, RAM, OS) so future comparisons are fair
And the entry MUST flag any regression > 25 % as a concern requiring a follow-up perf task
