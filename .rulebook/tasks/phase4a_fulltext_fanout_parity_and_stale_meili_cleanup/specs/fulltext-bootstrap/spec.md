# Fulltext fan-out parity & stale-index sweep spec

## ADDED Requirements

### Requirement: Fulltext worker replays missing-repo partitions on boot
The `cortex-fulltext` worker SHALL, on startup, reconcile the set of `(repo, family)` partitions present in the event archive with the set of Meilisearch indexes present, and replay events for any partition that has archive coverage but no Meili index. Replay MUST be idempotent — every Meili upsert uses an `id` derived from `content_hash`, so re-running the worker MUST NOT create duplicates.

#### Scenario: rulebook events present in archive, missing in Meili
Given the event archive under `~/.cortex/archive/events/` contains at least one event with `context_repo = "Rulebook"` and `kind = Artifact` mapping to the `code` family
And Meilisearch has no index named `cortex-rulebook-code`
When `cortex-fulltext` boots
Then the worker MUST create the `cortex-rulebook-code` index with the canonical settings (filterable + sortable attributes per spec-08)
And the worker MUST upsert at least one document into that index
And re-booting the worker a second time MUST NOT change `numberOfDocuments`

#### Scenario: cortex partition already current
Given Meili already contains `cortex-cortex-code` with `numberOfDocuments > 0` matching the archive's cortex/code events
When `cortex-fulltext` boots
Then the worker MUST NOT delete or recreate `cortex-cortex-code`
And the document count MUST remain monotonic (no decrease)

### Requirement: Stale-index sweep removes empty indexes that violate the naming scheme
The `cortex-fulltext` worker SHALL, on startup, sweep Meilisearch indexes and delete any whose name does NOT match the regex `^cortex-[a-z0-9][a-z0-9_-]*-(code|docs|decisions|turns|governance|misc)$` AND whose `numberOfDocuments` equals zero. Indexes that fail the regex but are non-empty MUST NOT be deleted; the worker MUST log a `warn` line naming the index and continue.

#### Scenario: empty stale index is deleted
Given Meili contains an empty index named `cortex-code` (no repo slug)
When `cortex-fulltext` boots
Then the worker MUST issue `DELETE /indexes/cortex-code`
And the worker MUST log `info` with the dropped name and the reason `stale-naming`

#### Scenario: non-empty stale index is preserved with a warning
Given Meili contains a non-empty index whose name violates the regex (e.g. `legacy-foo`)
When `cortex-fulltext` boots
Then the worker MUST NOT delete the index
And the worker MUST emit exactly one `warn` log line per offending index, including the name and `numberOfDocuments`

### Requirement: Routing invariant — three-token index names
`cortex_fulltext::routing::index_name` SHALL produce names that split on `-` into exactly three non-empty tokens (`cortex`, `{repo_slug}`, `{family}`). A `debug_assert!` MUST enforce the invariant in debug builds; a property test MUST cover random valid `(repo_slug, family)` inputs.

#### Scenario: index_name preserves the three-token shape
Given any `repo_slug` matching `[a-z0-9][a-z0-9_-]*`
And any `family` from the canonical list
When `index_name("cortex-", repo_slug, family)` is called
Then the returned string MUST split into exactly three tokens on `-`
And token[0] MUST equal `"cortex"`
And token[2] MUST equal `family`
