# Spec: Query lane wiring

## ADDED Requirements

### Requirement: decision_lookup MUST consult the decisions lane

`POST /v1/query` with `intent=decision_lookup` MUST query
Vectorizer `cortex.decision.fp32`, Meili `cortex_decisions`, and
Nexus `:Decision`, fuse via RRF, and return the result under
`results.decisions`.

#### Scenario: ADR body keyword hits even when title doesn't
Given the ADR `Bypass vectorizer-sdk for /insert` exists with the
  rationale body containing "RRF fusion algorithm choice rationale"
When the operator queries `intent=decision_lookup, query="RRF fusion
  algorithm choice rationale"`
Then `results.decisions` MUST contain that ADR
And the rank MUST be ≤ 10.

### Requirement: law_check MUST consult the laws + violations lane

`POST /v1/query` with `intent=law_check` MUST return at least
`results.laws` (matched law rows) plus `results.violations`
(matched law-violation events). Each law row MUST carry a body
excerpt large enough to quote the prohibition (≥ 256 chars).

#### Scenario: existing law surfaces by keyword
Given `GIT-SAFETY` is registered with body "Forbidden: git stash, git
  rebase, git reset --hard, …"
When the operator queries `intent=law_check, query="git safety
  prohibitions destructive"`
Then `results.laws` MUST contain `GIT-SAFETY`
And the response body MUST include the prohibition list excerpt.

### Requirement: similar_problems MUST consult the turns lane

`POST /v1/query` with `intent=similar_problems` MUST query
Vectorizer `cortex.turn.fp32`/`pq` and Meili `cortex_turns`,
return rows under `results.similar_turns`, and include
`session_id` + `occurred_at` so the caller can deep-link.

#### Scenario: prior turn about meili reset surfaces
Given a turn captured 5 days ago with body "I reset the meili index
  after the divergence alert"
When the operator queries `intent=similar_problems, query="previous
  turns about meili index reset"`
Then `results.similar_turns` MUST contain that turn
And the row MUST carry `session_id` + `occurred_at`.

### Requirement: lane-set table covers every intent

`crates/cortex-api/src/strategies.rs` MUST declare a complete
`intent → lanes` mapping for all five live intents
(`pre_change_context`, `decision_lookup`, `similar_problems`,
`law_check`, `free_search`). An intent missing from the table MUST
fail the orchestrator's compile-time check.

#### Scenario: relevance harness recall floor
Given the labelled query set in `tests/relevance/queries.toml`
When the relevance harness runs against the post-fix daemon
Then recall@10 MUST be > 0 for every intent bucket
And the global recall@10 MUST be ≥ 30 %.
