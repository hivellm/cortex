# Symbols & DEFINES edges spec

## ADDED Requirements

### Requirement: Symbol nodes mirror code-chunk symbols
The `cortex-graph` mapper SHALL emit a `Symbol` node for every artifact-chunk event whose `source == "code"` AND whose `symbol` payload field is non-empty. The natural key MUST be `(repo, language, qualified_name)`; when no qualified name is available, the key MUST fall back to `(repo, path, name)` to preserve uniqueness across files.

#### Scenario: code chunk with symbol field produces a Symbol node
Given an artifact-chunk event with `source = "code"`, `language = "rust"`, `symbol = "PreThinkingTool"`, `path = "crates/cortex-mcp-server/src/tools.rs"`, `repo = "Cortex"`
When the mapper processes the event
Then the resulting Patch MUST contain a `MergeNode` for label `Symbol` with `name = "PreThinkingTool"`, `language = "rust"`, `repo = "Cortex"`
And the natural key MUST be deterministic across reruns of the same event

#### Scenario: chunk without symbol stays Artifact-only
Given an artifact-chunk event whose `symbol` field is missing or empty
When the mapper processes the event
Then the resulting Patch MUST contain only the existing Artifact + IN_REPO patches (no `Symbol` node)
And the mapper MUST NOT log an error

### Requirement: DEFINES edge connects Symbol to Artifact
For every emitted `Symbol`, the mapper SHALL also emit a `DEFINES` edge from the `Symbol` to the `Artifact` it lives in. The edge MUST be MERGE-idempotent (re-running on the same chunk MUST NOT create a duplicate edge).

#### Scenario: replay does not duplicate DEFINES
Given the same artifact-chunk event has been processed twice
When `MATCH (s:Symbol {name: "PreThinkingTool"})-[r:DEFINES]->(a:Artifact) RETURN count(r)` is run
Then the count MUST equal `1`

#### Scenario: missing endpoint is tolerated
Given the `MATCH` for the Artifact endpoint of a `DEFINES` MERGE returns no rows (race or pruning)
When the writer executes the edge MERGE
Then the writer MUST NOT crash
And the writer MUST log at `debug` (not `error`) noting the dropped edge with the symbol key

### Requirement: Backfill is idempotent and safe
A re-run of `cortex-graph` against the existing event archive after the mapper change SHALL populate `Symbol` nodes and `DEFINES` edges for every code chunk previously stored as an Artifact, WITHOUT mutating existing Artifact nodes, IN_REPO edges, or any other existing graph state.

#### Scenario: backfill leaves prior counts monotonic
Given the prior graph has `N` Artifact nodes and `M` IN_REPO edges
When the worker replays the archive after the mapper change
Then the post-replay graph MUST have at least `N` Artifact nodes (no decrease)
And the post-replay graph MUST have at least `M` IN_REPO edges (no decrease)
And the post-replay graph MUST have at least one `Symbol` node and at least one `DEFINES` edge
