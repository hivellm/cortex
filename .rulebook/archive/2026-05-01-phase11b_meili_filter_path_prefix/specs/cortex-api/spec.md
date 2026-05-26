# cortex-api — Meili filter path prefix

## ADDED Requirements

### Requirement: Path prefix filter uses `path_prefixes IN [...]`
The Meili keyword lane in `cortex-api` SHALL filter `scope.files` prefixes via a `path_prefixes IN [...]` clause against a filterable array field, NEVER via the unsupported `STARTS WITH` operator.

#### Scenario: Single file prefix
Given a `cortex_query` request with `scope.files = ["crates/cortex-api/src/"]`
When `meili_lane::build_filter` runs
Then the emitted filter contains `(path_prefixes IN ['crates/cortex-api/src/'])`
And the literal string `STARTS WITH` does not appear anywhere in the filter

#### Scenario: Multiple file prefixes
Given `scope.files = ["a/b.rs", "c/d/"]`
When `build_filter` runs
Then the emitted filter contains `(path_prefixes IN ['a/b.rs', 'c/d/'])`

#### Scenario: Composes with other clauses via AND
Given `scope.files` is non-empty AND `scope.repo` is set
When `build_filter` runs
Then the path-prefix clause is wrapped in parens
And it is joined with other clauses by ` AND `

### Requirement: Indexed documents carry `path_prefixes`
The fulltext indexer SHALL populate a filterable array field `path_prefixes` on every document, containing every ancestor path plus the full path.

#### Scenario: Deep nested file
Given an envelope with `path = "crates/cortex-api/src/meili_lane.rs"`
When the fulltext worker projects it
Then the indexed document has `path_prefixes = ["crates/", "crates/cortex-api/", "crates/cortex-api/src/", "crates/cortex-api/src/meili_lane.rs"]`

#### Scenario: Single-segment path
Given an envelope with `path = "README.md"`
When the worker projects it
Then `path_prefixes = ["README.md"]`

#### Scenario: Empty / missing path
Given an envelope with no `path`
When the worker projects it
Then `path_prefixes = []` (or the field is omitted)

### Requirement: Index settings declare `path_prefixes` filterable
The Meili settings file SHALL include `path_prefixes` in `filterableAttributes` and the tooling-only `version` marker SHALL be bumped so the existing settings-version watcher triggers re-indexing.

#### Scenario: Settings version bump triggers re-index
Given the live index reports `version = "v1"` in its loader-side meta
And the new `settings.v1.json` declares a higher version
When the worker boots
Then it walks the archive and re-indexes every envelope into the per-project index
And every freshly-indexed document carries the new `path_prefixes` field
