# Rulebook artifact indexer spec

## ADDED Requirements

### Requirement: Indexer emits canonical envelopes for each .rulebook artifact kind
The system SHALL walk `.rulebook/decisions/`, `.rulebook/learnings/`, `.rulebook/knowledge/patterns/`, `.rulebook/knowledge/anti-patterns/`, and `.rulebook/specs/` from the active repo root and emit one canonical envelope per artifact, using the kinds `decision`, `learning`, `pattern`, and `law` respectively.

#### Scenario: decision file produces a Kind::Decision envelope
Given `.rulebook/decisions/001-bypass-vectorizer-sdk.md` exists with a sibling `.metadata.json`
When the indexer walks the directory
Then the indexer MUST emit exactly one envelope of `kind = "decision"`
And the envelope's payload MUST carry `id`, `title`, `status`, `ts`, `links`, and the full markdown body
And the envelope MUST flow through the canonical publisher (no direct lane writes)

#### Scenario: spec file with multiple requirements produces multiple Kind::Law envelopes
Given `.rulebook/specs/RULEBOOK.md` contains 5 `### Requirement:` headings
When the indexer parses the file
Then the indexer MUST emit 5 envelopes of `kind = "law"`
And each envelope MUST carry `id` (synthesised when absent), `title`, `severity` (defaulted from spec metadata), and the requirement body

#### Scenario: anti-pattern emits Kind::Pattern with discriminator
Given `.rulebook/knowledge/anti-patterns/cypher-unwind-write.md` exists
When the indexer parses the file
Then the indexer MUST emit one envelope of `kind = "pattern"`
And the envelope payload MUST set `pattern_kind = "anti_pattern"`

### Requirement: Indexer is idempotent across re-scans
The system SHALL deduplicate envelopes by `(kind, payload.id)` so re-running the walker on the same `.rulebook/` tree never inflates the lane.

#### Scenario: walker runs twice produces the same lane state
Given the indexer walks the tree at T0 and seeds the lane with N hits
When the indexer walks the tree again at T1 with no `.rulebook/**` changes
Then the lane MUST contain exactly N hits (no duplicates)

#### Scenario: edited artifact replaces the prior envelope
Given an artifact `001-bypass-vectorizer-sdk.md` was indexed at T0
When the file body changes and the indexer re-walks at T1
Then the lane's hit for `(decision, 001)` MUST reflect the new body
And no orphan / stale copy MUST remain

### Requirement: Query API consumes the new kinds
The `cortex-api` orchestrator SHALL surface `Kind::Decision` envelopes under `results.decisions` and `Kind::Law` envelopes under `laws_active` for any query whose `intent` is `pre_change_context`, `decision_lookup`, or `law_check`.

#### Scenario: pre_change_context returns at least one decision when ADRs exist
Given the rulebook indexer has emitted at least one `Kind::Decision` envelope into the lane
And `cortex-api` is running with the live lane wiring
When a `/v1/query` arrives with `intent: pre_change_context` and a non-empty `query`
Then `results.decisions` in the response MUST contain at least one entry referencing the indexed ADR id
And `laws_active` MUST be non-empty when at least one `Kind::Law` envelope matches the active repo scope
