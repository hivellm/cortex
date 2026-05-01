# cortex-api — vector lane search payload drift fix

## ADDED Requirements

### Requirement: Vector lane reads from the server's wire `payload`
The cortex-api vector lane SHALL parse `POST /collections/{c}/search/text` responses against the **real server wire shape** (`{id, score, vector, payload}`), NOT the `vectorizer-sdk` `SearchResult` shape (`{id, score, content, metadata}`) which silently drops every server-side payload field.

#### Scenario: Server returns rich payload → LaneHit carries text and path
Given the Vectorizer server responds with `{id, score, vector, payload: {path, body, kind, repo, ...}}`
When the lane projects the response
Then `LaneHit.path = payload.path`
And `LaneHit.text = payload.body` (or `summary`/`title` per the existing fallback chain)
And `LaneHit.repo`, `LaneHit.symbol`, `LaneHit.severity`, `LaneHit.ts` are populated from the payload

#### Scenario: Legacy nested payload still resolves
Given an older embedder build wrote `{payload: {payload: {path: ...}}}` (phase6b nesting)
When the lane projects the response
Then the projection finds `path` via the nested `payload.payload.path` fallback
And the resulting `LaneHit.path` is non-empty

#### Scenario: Empty body collapses to header-only (phase10b §1)
Given the wire payload's `body` and `summary` and `title` are all empty
When the lane projects the response
Then `LaneHit.text` is empty
And `extras.body_truncated = true` only when the upstream collapsed text into the path

### Requirement: Auth path stays on the SDK
The lane SHALL keep `probe_authenticated`, `refresh_token`, `health_check`, and `login` on the SDK because their wire shapes match.

#### Scenario: 401 → refresh → retry uses the same shape
Given a search request returns 401
When the lane refreshes the JWT
Then it retries the request through the same direct HTTP path (NOT the SDK)
And the retry's `Authorization: Bearer` header carries the freshly-minted JWT

### Requirement: Wire-shape regression tests
The integration test suite SHALL exercise the real wire shape with `wiremock` and assert non-empty `text` and `path` on the resulting `LaneHit`.

#### Scenario: Test mounts the real shape
Given a `wiremock` mount on `POST /collections/{c}/search/text`
When the test responds with `{results: [{id, score, vector, payload: {path: "src/lib.rs", body: "fn foo() {}", kind: "Artifact", repo: "Cortex"}]}`
Then the lane returns one `LaneHit` with `text == "fn foo() {}"`, `path == Some("src/lib.rs")`, `repo == Some("cortex")`

#### Scenario: Test catches a regression to the SDK shape
Given a `wiremock` mount that responds with the legacy SDK shape (`metadata` instead of `payload`)
When the lane projects
Then the resulting `LaneHit.text` is empty AND `LaneHit.path` is None
And the regression test fails (proves the new path actually parses `payload`)
