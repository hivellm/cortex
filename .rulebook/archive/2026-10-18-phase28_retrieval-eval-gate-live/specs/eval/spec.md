# Standalone local evaluation

## ADDED Requirements

### Requirement: Standalone prerequisite and offline execution

Implementation SHALL wait for phase36_local-first-core completion. Required
evaluation SHALL run explicitly and in push/PR CI over versioned local fixtures,
without Docker, database services, HTTP daemon, provider credentials or
external/OS schedules. This supersedes the nightly-live-retrieval requirement.

#### Scenario: Prerequisite incomplete
Given phase36_local-first-core has not completed acceptance
When this task is selected
Then it remains pending its prerequisite and later checklist items do not start.

#### Scenario: Clean offline execution
Given the accepted standalone build and public fixture corpus
When evaluation runs with external services unavailable
Then production import/reindex builds an isolated home and emits a complete report
without credentials or legacy services.

### Requirement: Recall checks identity and provenance

The corpus SHALL pin canonical hashes, stable identities, provenance and relevance
judgments. Recall@5 and MRR@10 SHALL report per-query failures, held-out cases and
intent coverage. Historical answers SHALL expose readable evidence.

#### Scenario: Counts cannot establish recall
Given expected record identities and source provenance
When retrieval returns equally many unrelated records
Then the report scores missed answers instead of treating result count as recall.

#### Scenario: Historical recall survives rebuild
Given historical turn/tool records with stable original source identities
When the derived index is rebuilt and queries repeat
Then identities, readable evidence and provenance remain equivalent
and original source hashes remain unchanged.

### Requirement: Exact scope applies to indirect observations

Scope isolation SHALL cover records, links, traversal, caches, statistics and
ranking inputs. Any cross-scope disclosure SHALL fail independently of quality scores.

#### Scenario: Warm caches do not cross scope
Given repositories sharing names/text but containing distinct evidence
When queries alternate between them with warm process caches
Then neither answers nor statistics or graph results reveal the other scope.

### Requirement: Graph quality checks supported answers

Evaluation SHALL verify expected nodes, typed edges and supporting evidence,
including bounded paths where supported. Counts alone SHALL NOT establish quality;
unsupported operations SHALL NOT be advertised or counted as evaluated.

#### Scenario: Wrong edge fails despite equal counts
Given expected typed relationships and source records
When an answer returns the same count with wrong endpoints or unsupported evidence
Then evaluation fails and identifies the incorrect relationship.

### Requirement: Jev offline and live evidence stay distinct

Offline fixtures SHALL test typed response handling, scoring, unknown/transient
outcomes and authority preservation. Live quality SHALL require explicit opt-in,
reviewed public payloads, bounded requests/time/cost, and model/schema/usage reports.
Credentials SHALL NOT appear in artifacts. Live runs SHALL NOT gate offline checks.

#### Scenario: Provider unavailable
Given no provider connection or credentials
When offline validation runs
Then deterministic checks complete without provider requests or live-quality claims.

#### Scenario: Live provider failure
Given an opted-in public-corpus run
When Jev times out or returns malformed probabilities
Then the report records failure without changing source authority
or replacing the failed observation with mocked success.

### Requirement: Auditable latency and regression floors

CLI, direct MCP and supported hooks SHALL report end-to-end p50/p95, bounded
samples/concurrency, cold/warm separation, build/corpus/hardware identity and
failure accounting. Reviewed baselines and justified floors SHALL precede
enforcement. Fixture/floor changes SHALL retain previous evidence and justification.
Prepared context SHALL NOT establish observed model use.

#### Scenario: Regression crosses a floor
Given recorded standalone baselines and acceptance floors
When measured quality or latency breaches a floor
Then validation fails with diagnostics and retains the failed report
without silently adjusting the floor.

### Requirement: Installed read-only surface equality

Smoke validation SHALL use an isolated fixture home, record binary identity and
compare advertised, implemented and documented tools plus CLI/MCP identities, provenance and scope
with the tested build. It SHALL NOT capture into the operator store or classify.

#### Scenario: Stale installed executable
Given the tested build's implemented tool set and fixture answers
When an installed executable advertises different tools or returns different
scoped identities/provenance
Then smoke validation fails with the installed build and exact mismatch.
