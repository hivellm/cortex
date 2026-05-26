# Per-project collection isolation spec

## ADDED Requirements

### Requirement: Collection / index names embed the repo slug
Every Vectorizer collection and Meilisearch index that holds repo-scoped content SHALL carry the owning repo's slug in its name. The format is `cortex-{repo_slug}-{family}`.

#### Scenario: bootstrapping the Cortex repo creates per-repo collections
Given the cortex-bootstrap CLI runs against `E:/HiveLLM/Cortex` with `cortex.id = "Cortex"`
And both Vectorizer and Meili are empty
When the embedder + fulltext workers consume the resulting envelopes
Then Vectorizer MUST contain at least one collection named `cortex-cortex-docs`
And Meili MUST contain at least one index named `cortex-cortex-docs`
And no collection / index without the `cortex-cortex-` prefix MAY exist for content emitted by this run

#### Scenario: bootstrapping a second repo does not pollute the first
Given the Cortex repo was bootstrapped at T0 producing collections under `cortex-cortex-`
When the cortex-bootstrap CLI later runs against `E:/HiveLLM/Tml` with `cortex.id = "Tml"`
Then any new collections / indexes MUST live under `cortex-tml-`
And the existing `cortex-cortex-*` collections MUST NOT receive any vector or document from the Tml run

### Requirement: Slug derivation is deterministic and identifier-safe
The system SHALL derive `repo_slug` deterministically from `cortex.id` (or the git-root basename when `cortex.id` is absent), producing a value that matches `[a-z0-9-]+` with no leading or trailing `-`.

#### Scenario: ASCII id with no special chars
Given a `cortex.id = "Cortex"`
When `slug_for_repo` is called
Then the result MUST equal `"cortex"`

#### Scenario: PascalCase id
Given a `cortex.id = "CompressionPrompt"`
When `slug_for_repo` is called
Then the result MUST equal `"compressionprompt"` (lowercase, no separator inserted)

#### Scenario: id contains slashes / spaces / accents
Given a `cortex.id = "Hive Hub/cloud"`
When `slug_for_repo` is called
Then the result MUST contain only `[a-z0-9-]`
And no leading or trailing `-` MAY appear

#### Scenario: empty / missing id
Given the slug helper receives an empty string
When `slug_for_repo("")` is called
Then the result MUST equal `"unknown"` so downstream collection naming never produces `cortex--family`

### Requirement: Read path scopes lane requests by repo
The orchestrator SHALL derive the target collection / index from `req.scope.repos`. When `scope.repos` contains a single repo, the orchestrator MUST query `cortex-{slug}-{family}` for that repo.

#### Scenario: scoped query hits the per-repo lane
Given a `/v1/query` arrives with `scope.repos = ["Cortex"]` and `intent: pre_change_context`
When the orchestrator builds the keyword-lane request
Then the request's `index` field MUST equal `cortex-cortex-{family}` for the appropriate family
And the request MUST NOT target the legacy `cortex-{family}` name

#### Scenario: unscoped query lands on `unknown`
Given a `/v1/query` arrives with `scope.repos = []`
When the orchestrator builds lane requests
Then each request's `index` / `collection` MUST equal `cortex-unknown-{family}`
And the response MAY be empty until the multi-repo fan-out lands in a follow-up task
