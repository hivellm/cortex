# Consistency doctor spec

## ADDED Requirements

### Requirement: Doctor reports per-(repo, family) coverage across all backends
`cortex doctor consistency` SHALL probe Meilisearch, Vectorizer, Nexus, and the local event archive and emit a per-`(repo, family)` row containing the document/vector/artifact count from each source. The output MUST be deterministic for a fixed input state.

#### Scenario: cortex/code partition fully populated
Given Meili has `cortex-cortex-code` with 342 docs
And Vectorizer has `cortex-cortex-code` with 10,352 vectors
And Nexus has `(:Repo {name:"Cortex"})` with 2,000 IN_REPO artifacts
And the archive has 1,500 cortex/code events
When `cortex doctor consistency` runs in coverage mode
Then the row for `(Cortex, code)` MUST report `archive=1500, vec=10352, meili=342, nexus=N/A` (Nexus is repo-grain)
And the row MUST NOT be marked inconsistent (all positive)
And the exit code MUST be `0`

#### Scenario: rulebook missing from Meili
Given Vectorizer has `cortex-rulebook-code` with 1,366 vectors
And Meili has no `cortex-rulebook-code` index
And the archive has rulebook/code events
When `cortex doctor consistency` runs in coverage mode
Then the row for `(Rulebook, code)` MUST be marked inconsistent
And the exit code MUST be non-zero
And the failure list MUST name `(Rulebook, code, meili=0)`

### Requirement: Probe mode measures cross-lane query overlap
The `--query <q>` flag SHALL run the same text query against each lane and compute the Jaccard overlap of the top-K result paths between every lane pair. A configurable floor `min_overlap_jaccard` enforces a minimum overlap threshold.

#### Scenario: well-indexed query exceeds threshold
Given the indexes are populated for the term "classifier worker"
And the doctor config sets `min_overlap_jaccard = 0.2`
When `cortex doctor consistency --query "classifier worker"` runs
Then the report MUST contain pairwise Jaccards for (vec↔meili), (vec↔nexus), (meili↔nexus)
And EVERY pairwise Jaccard MUST be `>= 0.2`
And the exit code MUST be `0`

#### Scenario: drift below threshold fails the run
Given the same query
And the configured floor is `0.2`
And the actual top-K overlap between vec and meili is `0.05`
When the doctor runs
Then the run MUST exit non-zero
And the report MUST name the offending pair and the actual Jaccard value

### Requirement: Doctor is read-only
The doctor SHALL NOT issue any write, delete, or update to any backend. The doctor MUST NOT trigger reindexing, re-bootstrap, or any side effect beyond reading.

#### Scenario: backend state is unchanged after doctor run
Given a snapshot of `numberOfDocuments` per Meili index, `vector_count` per Vectorizer collection, and `MATCH (n) RETURN count(n)` from Nexus
When `cortex doctor consistency` runs (in any mode)
Then the same snapshots taken immediately after the run MUST equal the pre-run snapshots

### Requirement: CI integration
The doctor SHALL be invokable from CI via `make doctor` (or a documented `cargo run` form) and MUST exit with a non-zero status code on any inconsistency. The doctor's JSON output (`--json`) MUST conform to a documented schema so downstream workflows can parse it.

#### Scenario: CI gate blocks merge on regression
Given a pull request introduces a regression that empties the `cortex-rulebook-code` Meili index
When the CI workflow runs `make doctor` after `docker-compose up -d` and a seeded bootstrap
Then the workflow step MUST fail
And the JSON output MUST include the `(Rulebook, code)` row marked inconsistent
