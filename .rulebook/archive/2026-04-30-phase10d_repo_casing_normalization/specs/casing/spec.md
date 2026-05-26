# Spec: Repo casing normalization

## ADDED Requirements

### Requirement: Canonical repo case is lowercase

Every event envelope persisted past phase10d MUST carry `repo`
lowercased. The original-case display string MUST flow as
`repo_label` so the GUI can show `Cortex` while the wire carries
`cortex`.

#### Scenario: walker lowercases at emission
Given a directory `/repos/Cortex` is bootstrapped
When the walker emits envelopes
Then every envelope's `payload.repo` MUST equal `cortex`
And every envelope's `payload.repo_label` MUST equal `Cortex`.

### Requirement: Case-insensitive scope matching

The query orchestrator MUST treat `scope.repo` case-insensitively.
A query with `scope.repo: "Cortex"` MUST resolve identically to
one with `scope.repo: "cortex"`.

#### Scenario: capitalised scope hits the same rows
Given the lane carries 100 events tagged `repo=cortex`
When the operator queries `intent=free_search, scope.repo="Cortex"`
Then the response MUST contain the same hits as a query with
  `scope.repo="cortex"`
And the audit envelope's `scope_resolved.repo` MUST be `cortex`.

### Requirement: Relevance harness omits zero buckets

After the one-shot canonicalize CLI applies, the canonical query
fixture (`tests/relevance/queries.toml`) MUST execute end-to-end
without any bucket being marked omitted.

#### Scenario: harness covers every intent
Given the canonicalize CLI ran with `--apply`
When the relevance harness runs against the post-fix daemon
Then `omitted_intents` in the report MUST be empty
And every intent MUST report a `queries` count > 0.
