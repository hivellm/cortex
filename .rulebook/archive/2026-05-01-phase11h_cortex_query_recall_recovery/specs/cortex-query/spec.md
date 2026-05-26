# Spec: cortex-query recall recovery

## ADDED Requirements

### Requirement: Daemon image deploys current HEAD

The `cortex-api` container SHALL run a binary built from the
current git HEAD before any other §1 task is marked complete.

#### Scenario: healthz reports current SHA

Given the operator rebuilds and restarts the `cortex-api` container
When the operator calls `GET /healthz`
Then `extras.version.git_sha` MUST equal `git rev-parse HEAD`
And `extras.version.git_dirty` MUST be `false`
And `extras.version.build_ts` MUST NOT equal `"unknown"`

#### Scenario: keyword lane no longer rejects scope.files

Given the redeployed daemon
When a caller fires `pre_change_context` with `scope.files=["crates/cortex-api/src/main.rs"]`
Then the keyword lane MUST NOT return `invalid_search_filter`
And the emitted Meili filter MUST use `path_prefixes IN [..]` shape
And MUST NOT contain the substring `STARTS WITH`

#### Scenario: free_search response stays under MCP transport cap

Given the redeployed daemon
When a caller fires `free_search` with a high-recall query that would otherwise produce >100 KiB of hits
Then the wire response size MUST be ≤ 32 KiB by default
And the response MUST carry a non-null `clipped` field describing what was removed
And the MCP server MUST NOT dump the response to a side-file

### Requirement: Bootstrap fills every (repo, family) target

The bootstrap pipeline SHALL produce one collection per
`indexed_repo × family` combination on both backends; no legacy
indexes from earlier naming schemes SHALL remain.

#### Scenario: coverage reaches ok severity

Given a complete bootstrap run for every entry in `indexed_repos`
When the operator calls `GET /v1/health/coverage`
Then for both backends `present_count` MUST equal `expected_count`
And `missing_count` MUST equal `0`
And `unexpected_count` MUST equal `0`
And `overall_severity` MUST equal `"ok"`

#### Scenario: legacy unprefixed meili indexes removed

Given the seven legacy `cortex-{family}` (no repo prefix) indexes
When the operator confirms via grep that no current code path reads them
And the operator deletes them via authenticated `DELETE /indexes/{name}`
Then `/v1/health/coverage.backends[meili].unexpected` MUST equal `[]`

### Requirement: ADRs and laws are queryable through cortex-api

Every ADR under `.rulebook/decisions/*.md` and every behavioral
law in `AGENTS.override.md` / `.claude/rules/*.md` SHALL be
ingested into the appropriate cortex lane and SHALL be retrievable
through the corresponding query intent.

#### Scenario: decision_lookup retrieves an ADR

Given an ADR file `001-bypass-vectorizer-sdk.md` exists in `.rulebook/decisions/`
And the bootstrap has ingested it into `cortex-cortex-decisions`
When a caller fires `decision_lookup` with `query="bypass vectorizer SDK"` and `scope.repo="cortex"`
Then `results.decisions` MUST contain ≥ 1 entry
And the matched entry's path MUST resolve back to that ADR file

#### Scenario: law_check retrieves LAW-CORTEX-001

Given `LAW-CORTEX-001` is defined in `AGENTS.override.md`
And the bootstrap has ingested it into the governance lane
When a caller fires `law_check` with `query="task sequence cherry pick"` and `scope.repo="cortex"`
Then `results.violations` (or the law-shaped equivalent in this intent) MUST contain LAW-CORTEX-001
And the entry MUST carry the law's severity and a rationale excerpt

#### Scenario: ingestion pipeline auto-republishes on file change

Given an ADR file is added or modified under `.rulebook/decisions/`
When the bootstrap-time scanner (or live watcher) next runs
Then the change MUST be reflected in the next `decision_lookup`
without operator-side manual republishing

### Requirement: CI gates against regression of any of the above

Three integration tests SHALL guard the recovery so it does not
silently drift back out of compliance.

#### Scenario: coverage_drift_it fails when a backend regresses

Given the daemon is running with at least one missing or unexpected index
When `cargo test -p cortex-api --test coverage_drift_it` runs with `CORTEX_COVERAGE_IT=1`
Then the test MUST fail
And the failure message MUST name the offending backend and counts

#### Scenario: intent_smoke_it covers every intent

Given the seeded fixture corpus
When `cargo test -p cortex-mcp-server --test intent_smoke_it` runs
Then one assertion per intent (`free_search`, `pre_change_context`, `decision_lookup`, `law_check`, `similar_problems`) MUST pass
And every response MUST be ≤ 32 KiB

#### Scenario: healthz_release_it pins build metadata

Given a release-profile build
When `cargo test -p cortex-api --test healthz_release_it` runs
Then `git_sha`, `build_ts`, and `git_dirty` MUST all be non-default
