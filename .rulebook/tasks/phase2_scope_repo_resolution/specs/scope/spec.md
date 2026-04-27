# Scope repo resolution spec

## MODIFIED Requirements

### Requirement: Scope derivation always resolves a repo when cwd is inside one
The `cortex-pre-thinking::scope::derive` function SHALL emit a non-empty `scope.repos` whenever `cwd` is inside a git working tree, regardless of whether a `cortex.toml` exists.

#### Scenario: cwd inside git repo without cortex.toml
Given `cwd` is `/home/me/projects/sample-repo/sub/dir` and `/home/me/projects/sample-repo/.git` exists
And there is no `cortex.toml` in any ancestor
When `derive(prompt, cwd, &[])` runs
Then the returned `DerivedScope.scope.repos` MUST equal `["sample-repo"]`
And `repo_resolved` MUST be `true`

#### Scenario: cortex.toml id wins over basename
Given the same cwd and a `cortex.toml` in the repo root with `cortex.id = "Cortex"`
When `derive` runs
Then `scope.repos` MUST equal `["Cortex"]`

#### Scenario: cwd outside any git repo
Given `cwd` has no `.git` ancestor
When `derive` runs
Then `scope.repos` MAY be empty
And `repo_resolved` MUST be `false`

## ADDED Requirements

### Requirement: Strategies propagate scope to lanes
Every orchestrator strategy SHALL forward `req.scope.repos` into the lane request's `filter`. A lane that receives a non-empty repos filter MUST restrict its hits to those repos.

#### Scenario: keyword lane filtered to a repo
Given `req.scope.repos = ["Cortex"]`
And the live keyword lane is bound
When the orchestrator runs the strategy
Then the keyword lane's outgoing request MUST carry `filter` containing the equivalent of `repo IN ["Cortex"]`
And no hit from a different repo MAY appear in the response

### Requirement: Response echoes the canonical scope
`QueryResponse.scope_resolved.repos` SHALL contain the canonical repo ids actually used by the lanes' filters.

#### Scenario: echo matches filter
Given the orchestrator filtered hits with `repo = "Cortex"`
When the response is built
Then `scope_resolved.repos` MUST equal `["Cortex"]`
