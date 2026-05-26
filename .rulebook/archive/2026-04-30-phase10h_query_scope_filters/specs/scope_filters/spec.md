# Spec: Query scope filters

## ADDED Requirements

### Requirement: scope.since filters every lane

`POST /v1/query` with `scope.since=<RFC-3339>` MUST exclude every
row whose `occurred_at < since` from the response, regardless of
which lane produced it.

#### Scenario: 30-day cutoff drops older rows
Given the lane carries 1000 events spanning 90 days
When the operator queries with `scope.since="<now-30d>"`
Then every returned row MUST have `occurred_at >= now-30d`
And the `query_audit.scope_resolved.since` MUST equal the cutoff.

### Requirement: scope.topics filters every lane

`POST /v1/query` with `scope.topics=[t1, t2]` MUST return only
rows whose `payload.topic` (or `extras.classifier.top_topic`) is
in the list. Multiple topics OR.

#### Scenario: governance + law topics
Given the lane carries 100 events tagged `topic=governance`,
  100 tagged `topic=law`, 800 other
When the operator queries with `scope.topics=["governance","law"]`
Then exactly the 200 governance + law rows MUST be eligible
And no other-topic row MUST appear in the response.

### Requirement: scope.files extends to vector + graph

`POST /v1/query` with `scope.files=["prefix1/", "prefix2/"]` MUST
filter the keyword, vector, AND graph lanes — today only the
keyword lane honors it.

#### Scenario: file prefix narrows vector hits
Given the lane carries vectors for files in
  `crates/cortex-api/src/` and `crates/cortex-storage/src/`
When the operator queries with `scope.files=
  ["crates/cortex-api/src/"]`
Then no `cortex-storage` row MUST appear
And the response audit MUST carry the resolved files list.

### Requirement: query_audit carries the resolved scope

Every emitted `cortex.events.query_audit` envelope MUST include
`scope_resolved.{repo, files, since, topics}` exactly as the
lanes saw them. Missing input fields MUST be absent (not `null`)
in the audit so the dashboard's audit drawer renders cleanly.

#### Scenario: missing scope fields stay absent
Given a query without `scope.since` or `scope.topics`
When the audit envelope is emitted
Then the envelope MUST NOT carry `scope_resolved.since`
  or `scope_resolved.topics` keys
And it MUST carry only the populated fields.
