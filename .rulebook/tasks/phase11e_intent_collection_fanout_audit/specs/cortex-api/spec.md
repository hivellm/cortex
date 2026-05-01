# cortex-api / embedder / fulltext — intent-collection fan-out audit

## ADDED Requirements

### Requirement: Boot-time collection inventory diff
The cortex-api daemon SHALL diff every per-intent collection it expects to query against the live Vectorizer + Meili inventories at boot, logging a WARN per missing collection.

#### Scenario: All expected collections present
Given the live Vectorizer hosts every `cortex-{slug}-{kind}` collection in the routing matrix
When cortex-api boots
Then the boot log records `vector lane: collection coverage 100%`
And no WARN-level missing-collection log entries appear

#### Scenario: Missing per-kind collections
Given the live Vectorizer hosts only `cortex-cortex-{code,docs,misc,governance}`
And the routing matrix expects `cortex-cortex-{decisions,turns,memory,analyses,laws}` as well
When cortex-api boots
Then the boot log records WARN per missing collection with the slug, kind, and the dependent intents

### Requirement: Health surface exposes coverage
The `/v1/health` aggregator SHALL include per-lane `expected` / `present` / `missing` collection sets.

#### Scenario: Coverage section in /v1/health
Given the routing matrix expects N collections per slug
When a caller GETs `/v1/health`
Then the response includes `lanes.vector.coverage.{expected, present, missing}` with the same diff the boot log emitted
And the dashboard Health view renders the missing list as a tag cluster under the lane's extras column

### Requirement: Per-kind writer dispatch
The embedder + fulltext workers SHALL fan envelopes out to per-kind collections / indexes following the canonical naming `cortex-{slug}-{kind}` for `kind ∈ {code, docs, misc, governance, decisions, turns, memory, analyses, laws}`.

#### Scenario: Decision envelope writes to the decisions collection
Given a `cortex.events.enriched` envelope with `kind = "Decision"` and `repo = "Cortex"`
When the embedder consumes it
Then it upserts the chunk(s) into `cortex-cortex-decisions`
And the fulltext worker indexes the same envelope into `cortex-cortex-decisions` (Meili)

#### Scenario: Unknown kinds fall through to misc
Given an envelope whose `kind` does not match any declared dispatch
When the writer consumes it
Then it routes the chunk to `cortex-{slug}-misc`
And logs a `DEBUG` note so the gap is auditable

### Requirement: Optional missing-collection diagnostic on query
The orchestrator SHALL, when `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1`, surface missing-collection observations as `debug.notes[]` on the response.

#### Scenario: Diagnostic disabled (default)
Given `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS` is unset or `0`
When a `cortex_query decision_lookup` call hits a missing `*-decisions` collection
Then the response shape is unchanged (`results: {}`, no debug notes)
And the lane's `not found` swallow stays in place

#### Scenario: Diagnostic enabled
Given `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1`
When a `cortex_query decision_lookup` call hits a missing `cortex-cortex-decisions`
Then `debug.notes[]` includes an entry like `{lane: "vector", note: "collection_missing", collection: "cortex-cortex-decisions"}`

### Requirement: cortex_status reports honest per-backend coverage
The `cortex_status` MCP tool SHALL report `indexed_repos` in a way that distinguishes which backend (`vectorizer` / `meili` / `nexus`) actually has data for each repo.

#### Scenario: Single repo, multiple backends
Given the Vectorizer holds 4 collections all under `cortex-cortex-*`
And Meili holds indexes for `cortex` plus 14 other repos
And Nexus has graph data for `cortex` plus a subset
When `cortex_status` is invoked
Then the response carries `indexed_repos: { vectorizer: ["cortex"], meili: [..15..], nexus: [..N..] }` (or an equivalent shape that operators can read per-backend)
And the dashboard's banner reflects the per-backend coverage instead of a single conflated count
