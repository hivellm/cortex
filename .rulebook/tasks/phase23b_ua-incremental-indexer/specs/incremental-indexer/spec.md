# Incremental Indexer

## ADDED Requirements

### Requirement: Fingerprint-based staleness
The system SHALL persist a per-repo last-indexed commit hash and MUST treat the repo
as up-to-date (performing no graph work) when the stored hash equals the current
`HEAD`.

#### Scenario: Up-to-date repo is a no-op
Given a repo whose stored last-indexed hash equals current HEAD
When the indexer runs
Then no nodes, edges, or embeddings are written and the run completes as a no-op

#### Scenario: First run has no fingerprint
Given a repo with no stored last-indexed hash
When the indexer runs
Then it performs a full index and records the current HEAD as the fingerprint

### Requirement: Tiered change classification
The system SHALL classify a commit's changed-file set into one of `NOOP`,
`PARTIAL_UPDATE`, `ARCHITECTURE_UPDATE`, or `FULL_UPDATE`, using per-repo configurable
thresholds, and MUST gate architecture-level re-synthesis so it runs only for
`ARCHITECTURE_UPDATE` and `FULL_UPDATE`.

#### Scenario: Cosmetic change classifies NOOP
Given a commit that changes only comments and whitespace
When the classifier runs
Then it returns `NOOP` and the fingerprint advances with zero graph writes

#### Scenario: Architecture change invalidates synthesis
Given a commit that adds or removes directories
When the classifier returns `ARCHITECTURE_UPDATE`
Then architecture-level consolidation/topic-card synthesis is invalidated

### Requirement: Surgical merge preserves history
The system SHALL update the graph for changed files by bitemporal-closing the affected
nodes and edges (setting `valid_to`) rather than hard-deleting them, then upserting
freshly extracted nodes and edges with `valid_from = now`, and MUST be idempotent for
an unchanged repo.

#### Scenario: Re-run is byte-identical
Given a repo indexed at HEAD
When the indexer runs again with no new commits
Then the resulting graph is byte-identical to the prior state

#### Scenario: Rename rebinds identity
Given a commit that renames a file from `a` to `b`
When the merge runs
Then the file's node identity is rebound to `b` rather than deleted and recreated
