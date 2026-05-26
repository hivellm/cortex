# Proposal: phase4d_indexing_consistency_doctor

## Why

The 2026-04-27 22:36 UTC audit caught two structural drifts —
Meilisearch missing two repos out of three (phase4a) and the
graph carrying only two edge types out of the planned set
(phase4c) — only because the operator hand-curled the four
backends, decompressed the zstd-encoded event archive in Python,
and eyeballed a per-(repo, family) table.

That is not a sustainable detection method. There is no
automated check today that asserts:

- every repo present in any backend is present in **all** backends
- the per-`(repo, family)` document count between Vectorizer
  vectors / Meili docs / Nexus artifacts is within an expected
  ratio (vectorizer chunks more than 1× per file, but the order
  of magnitude should match)
- the event archive's `(repo, family)` partition list is fully
  reflected in each backend
- a same-query probe across the three lanes returns at least one
  overlapping `path` for queries that have indexed coverage

Without an automated doctor, the next regression after phase4a/b/c
will be discovered the same way — by accident, weeks later, after
pre-thinking bundles silently degrade.

## What Changes

- New subcommand `cortex doctor consistency` (extending the
  existing `cortex-ops` crate, since the dependency tree there
  already includes Vectorizer + Meili + Nexus clients).
- Two run modes:
  1. **Coverage mode** (default): for every repo present in any
     backend, list per-`(repo, family)` counts across
     Vectorizer / Meili / Nexus. Compare against the event-archive
     partition list. Output a markdown table and a JSON dump.
     Exit code is non-zero when a backend is missing a partition
     that another backend has.
  2. **Probe mode** (`--query <q> [--query <q2>...]`): for each
     query, run identical text searches against the three lanes
     and compute the Jaccard overlap of the top-K result paths.
     Report per-query overlap and a global average.
- Tolerance configuration via `cortex-doctor.toml`:
  - `min_overlap_jaccard`: floor for probe mode (e.g. 0.2)
  - `vec_to_meili_ratio_max`: a vectorizer-vs-meili count ratio
    upper bound — beyond it, warn (chunking can legitimately
    multiply, but 100× is suspicious)
- Output: human-readable table (default) plus a `--json` switch
  for CI.
- Wiring into CI: a new make target / GitHub Actions step that
  runs the doctor against the user's local stack post-bootstrap;
  failure blocks deploys.
- Doctor is **read-only**: no mutating calls to any backend.

The doctor is the test harness for phase4a/b/c. After each of
those tasks lands, running `cortex doctor consistency` proves the
fix landed at the data level — not just the unit-test level.

## Impact

- Affected specs: spec-13 (cortex-ops — adds doctor subcommand)
  or new spec-14 dedicated to consistency contracts.
- Affected code:
  - `crates/cortex-ops/src/doctor.rs` — new module
  - `crates/cortex-ops/src/cli.rs` — new `doctor consistency`
    subcommand
  - `crates/cortex-ops/src/main.rs` — wiring
  - new: `cortex-doctor.toml` template at the repo root
  - tests: unit tests against mocked backend responses; integration
    test against a docker-compose-up stack with seeded data
- Breaking change: NO. New subcommand only.
- User benefit: structural drift is caught the moment it appears
  instead of weeks later; CI gate prevents regressions; one
  command answers "are my three backends in sync?".

## Source

- Audit data captured 2026-04-27 22:36 UTC against running stack.
- Detection methodology used during the audit (manual curl + zstd
  decompress + Python reduction) is the anti-pattern this task
  removes.
